#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <bcrypt.h>
#include <io.h>
#include <fcntl.h>
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <functional>
#include <memory>
#include <stdexcept>
#include <thread>
#include "common.h"
#include "log.h"
#include "server-context.h"
#include "server-common.h"
#include <nlohmann/json.hpp>
struct Failure:std::runtime_error{using std::runtime_error::runtime_error;};
static void need(bool ok,const char* message){if(!ok)throw Failure(message);}
static void read(void* p,size_t n){need(std::fread(p,1,n,stdin)==n,"request-truncated");}
static void write(const void* p,size_t n){need(std::fwrite(p,1,n,stdout)==n&&std::fflush(stdout)==0,"response-write");}
static void data(const std::string& text){for(size_t at=0;at<text.size();at+=32768){const auto n=static_cast<uint32_t>(std::min<size_t>(32768,text.size()-at));write(&n,4);write(text.data()+at,n);}}
static void end(){const uint32_t zero=0;write(&zero,4);}
static void quiet(enum ggml_log_level,const char*,void*){}
struct Loop {
    server_context& context;
    std::thread thread;
    explicit Loop(server_context& value):context(value),thread([&value]{value.start_loop();}){}
    ~Loop(){context.terminate();if(thread.joinable())thread.join();}
};
struct Response {
    server_http_res_ptr value;
    ~Response(){if(value)value->on_complete();}
};
static std::string text(const std::wstring& value){const auto n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,value.data(),static_cast<int>(value.size()),nullptr,0,nullptr,nullptr);need(n>0,"utf8-path");std::string out(n,0);need(WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,value.data(),static_cast<int>(value.size()),out.data(),n,nullptr,nullptr)==n,"utf8-path");return out;}
static server_http_res_ptr invoke(server_routes& routes,const server_http_req& request,const std::string& method,uint32_t output_limit){
    if(method=="GET"&&request.path=="/health")return routes.get_health(request);
    if(method=="GET"&&request.path=="/v1/models")return routes.get_models(request);
    if(method=="POST"&&request.path=="/v1/chat/completions"){
        const auto body=json::parse(request.body);need(body.is_object(),"body-object");
        unsigned limits=0;for(const auto key:{"max_tokens","max_completion_tokens","n_predict"})if(body.contains(key)){need(body[key].is_number_integer(),"token-limit-integer");const auto maximum=body[key].get<int64_t>();need(maximum>0&&maximum<=output_limit,"max-tokens-bound");++limits;}
        if(body.contains("tools")){need(body["tools"].is_array(),"tools-array");for(const auto& tool:body["tools"])need(tool.is_object()&&tool.value("type",std::string())=="function","function-tools-only");}
        return routes.post_chat_completions(request);
    }
    if(method=="POST"&&request.path=="/apply-template")return routes.post_apply_template(request);
    auto response=std::make_unique<server_http_res>();response->status=404;response->data=R"({"error":{"message":"route unavailable"}})";return response;
}

struct TemplatePin {
    HANDLE handle=INVALID_HANDLE_VALUE;
    ~TemplatePin(){if(handle!=INVALID_HANDLE_VALUE)CloseHandle(handle);}
    std::string read(const wchar_t* path,const std::string& expected){
        need(expected.size()==64&&expected.find_first_not_of("0123456789abcdef")==std::string::npos,"template-sha-format");
        handle=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);need(handle!=INVALID_HANDLE_VALUE,"template-open");
        BY_HANDLE_FILE_INFORMATION info{};LARGE_INTEGER size{};need(GetFileInformationByHandle(handle,&info)&&GetFileSizeEx(handle,&size)&&!(info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT))&&size.QuadPart>0&&size.QuadPart<=131072,"template-file");
        std::string contents(static_cast<size_t>(size.QuadPart),0);DWORD count=0;need(ReadFile(handle,contents.data(),static_cast<DWORD>(contents.size()),&count,nullptr)&&count==contents.size(),"template-read");
        BCRYPT_ALG_HANDLE algorithm=nullptr;need(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)==0,"template-sha-provider");unsigned char digest[32]{};
        const auto status=BCryptHash(algorithm,nullptr,0,reinterpret_cast<PUCHAR>(contents.data()),count,digest,32);BCryptCloseAlgorithmProvider(algorithm,0);need(status==0,"template-sha");
        std::string actual;constexpr char hex[]="0123456789abcdef";for(auto byte:digest){actual+=hex[byte>>4];actual+=hex[byte&15];}need(actual==expected,"template-sha-mismatch");return contents;
    }
};

#include "model_launch_config.h"
#include "model_file_pin.h"
int wmain(int argc,wchar_t** argv){
    if(argc!=3&&argc!=5)return 64;
    try{
        need(_setmode(_fileno(stdin),_O_BINARY)!=-1&&_setmode(_fileno(stdout),_O_BINARY)!=-1,"stdio-binary");
        const bool configured=argc==5&&std::wstring(argv[1])==L"--config";TemplatePin config_pin;ModelLaunchConfig config;
        if(configured)config=parse_model_launch_config(config_pin.read(argv[2],text(argv[3])));
        const auto model=configured?config.model_path:text(argv[1]),nonce=text(argv[configured?4:2]);need(nonce.size()==32&&nonce.find_first_not_of("0123456789abcdef")==std::string::npos,"run-nonce");
        char intro[40];read(intro,sizeof(intro));need(std::memcmp(intro,"GENTSR01",8)==0&&std::memcmp(intro+8,nonce.data(),32)==0,"run-binding");
        common_log_set_verbosity_thold(-1);common_log_pause(common_log_main());llama_log_set(quiet,nullptr);ggml_log_set(quiet,nullptr);
        ModelFilePin model_pin;if(configured){const auto path=model_wide_path(model);model_pin.verify(path.c_str(),config.model_sha,16ull*1024*1024*1024);}
        llama_backend_init();
        common_params params;params.model.path=model;params.model_alias.insert(configured?config.model_id:"gent-contained-test");params.n_gpu_layers=-2;params.load_mode=LLAMA_LOAD_MODE_NONE;params.offline=true;
        params.n_ctx=configured?config.context:1024;params.n_batch=1024;params.n_ubatch=128;params.n_parallel=1;params.n_predict=configured?static_cast<int32_t>(config.output):64;params.cpuparams.n_threads=configured?static_cast<int>(config.threads):1;params.cpuparams_batch.n_threads=params.cpuparams.n_threads;
        TemplatePin template_pin;if(configured){params.cache_type_k=GGML_TYPE_Q8_0;params.cache_type_v=GGML_TYPE_Q8_0;params.enable_reasoning=config.thinking?1:0;params.sampling.reasoning_budget_tokens=static_cast<int32_t>(config.reasoning_budget);for(auto it=config.kwargs.begin();it!=config.kwargs.end();++it)params.default_template_kwargs[it.key()]=it.value().dump();if(!config.template_path.empty()){const auto path=model_wide_path(config.template_path);params.chat_template=template_pin.read(path.c_str(),config.template_sha);}}else if(argc==5)params.chat_template=template_pin.read(argv[3],text(argv[4]));
        params.use_jinja=true;params.server_tools.clear();params.server_tools_runtime.clear();
        server_context context;server_routes routes(params,context);need(context.load_model(params),"model-load");routes.update_meta(context);Loop loop(context);
        write("GENTSD01",8);write(nonce.data(),nonce.size());
        const std::function<bool()> stop=[] {return false;};
        for(unsigned count=0;count<32;++count){
            uint32_t length=0;read(&length,4);if(!length)break;need(length<=262144,"request-size");std::string encoded(length,0);read(encoded.data(),length);
            const auto envelope=json::parse(encoded);need(envelope.is_object()&&envelope.size()==3&&envelope.contains("method")&&envelope.contains("path")&&envelope.contains("body"),"request-envelope");
            const auto method=envelope.at("method").get<std::string>(),path=envelope.at("path").get<std::string>(),body=envelope.at("body").get<std::string>();need(path.size()<=128&&body.size()<=131072,"request-fields");
            server_http_req request{{},{},path,"",body,{},stop};Response response;
            try{response.value=invoke(routes,request,method,configured?config.output:64);}
            catch(const std::invalid_argument&){response.value=std::make_unique<server_http_res>();response.value->status=400;response.value->data=R"({"error":{"message":"invalid request"}})";}
            catch(const nlohmann::json::exception&){response.value=std::make_unique<server_http_res>();response.value->status=400;response.value->data=R"({"error":{"message":"invalid JSON request"}})";}
            need(response.value!=nullptr,"response-missing");
            const auto& value=*response.value;const auto metadata=json{{"status",value.status},{"contentType",value.content_type},{"headers",value.headers},{"stream",value.is_stream()}}.dump();need(metadata.size()<=32768,"response-metadata-size");data(metadata);
            size_t total=0;auto output=[&](const std::string& chunk){need(chunk.size()<=4*1024*1024-total,"response-size");total+=chunk.size();data(chunk);};
            if(value.is_stream()){bool more=true;while(more){std::string chunk;more=value.next(chunk);output(chunk);}}
            else output(value.data);
            end();
        }
        return 0;
    }catch(const Failure& error){std::fprintf(stderr,"gent-server-context: %s\n",error.what());return 65;}
    catch(const std::exception&){std::fprintf(stderr,"gent-server-context: failure\n");return 65;}
}

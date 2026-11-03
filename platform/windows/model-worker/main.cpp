#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <io.h>
#include <fcntl.h>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <stdexcept>
#include "llama.h"
#include "model_protocol_bounds.h"
// Bounded serial inference over inherited stdio; no networking implementation.
struct Failure:std::runtime_error{using std::runtime_error::runtime_error;};
static void need(bool ok,const char* stage){if(!ok)throw Failure(stage);}
static void quiet_log(enum ggml_log_level,const char*,void*){}
static void read(void* p,size_t n){need(std::fread(p,1,n,stdin)==n,"request-truncated");}
static void write(const void* p,size_t n){need(std::fwrite(p,1,n,stdout)==n&&std::fflush(stdout)==0,"response-write");}
static void frame(const std::string& p){const auto n=static_cast<uint32_t>(p.size());write(&n,4);if(n)write(p.data(),n);}
template<class F> static void bind(F& out,HMODULE m,const char* name){auto p=GetProcAddress(m,name);need(p!=nullptr,"runtime-export");static_assert(sizeof(out)==sizeof(p));std::memcpy(&out,&p,sizeof(p));}
#define FUNCTIONS(X) \
 X(llama_model_default_params) X(llama_context_default_params) X(llama_model_load_from_file) X(llama_model_get_vocab) \
 X(llama_init_from_model) X(llama_tokenize) X(llama_batch_get_one) X(llama_decode) X(llama_token_to_piece) \
 X(llama_vocab_is_eog) X(llama_sampler_init_greedy) X(llama_sampler_sample) X(llama_sampler_free) \
 X(llama_log_set) X(llama_free) X(llama_model_free) X(llama_get_memory) X(llama_memory_clear)
struct Api{
#define FIELD(n) decltype(&::n) n=nullptr;
 FUNCTIONS(FIELD)
#undef FIELD
 explicit Api(HMODULE module){
#define LOAD(n) bind(n,module,#n);
 FUNCTIONS(LOAD)
#undef LOAD
 }
};
static std::string utf8(const std::wstring& s){int n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,s.data(),static_cast<int>(s.size()),nullptr,0,nullptr,nullptr);need(n>0,"path-utf8");std::string v(n,0);need(WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,s.data(),static_cast<int>(s.size()),v.data(),n,nullptr,nullptr)==n,"path-utf8");return v;}
int wmain(int argc,wchar_t** argv){
 if(argc!=4)return 64;
 llama_model* model=nullptr;llama_context* context=nullptr;llama_sampler* sampler=nullptr;
 try{
  need(_setmode(_fileno(stdin),_O_BINARY)!=-1&&_setmode(_fileno(stdout),_O_BINARY)!=-1,"stdio-binary");
  const std::wstring runtime=argv[1];need(runtime.size()>3&&((runtime[0]>=L'A'&&runtime[0]<=L'Z')||(runtime[0]>=L'a'&&runtime[0]<=L'z'))&&runtime[1]==L':'&&runtime[2]==L'\\',"runtime-absolute");
  const auto nonce=utf8(argv[3]);need(nonce.size()==32&&nonce.find_first_not_of("0123456789abcdef")==std::string::npos,"run-nonce");
  char intro[40];read(intro,sizeof(intro));need(std::memcmp(intro,"GENTLM01",8)==0&&std::memcmp(intro+8,nonce.data(),32)==0,"run-binding");
  need(SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32|LOAD_LIBRARY_SEARCH_USER_DIRS)!=0,"dll-search-policy");
  need(AddDllDirectory(runtime.c_str())!=nullptr,"dll-runtime-directory");
  const auto base=LoadLibraryExW((runtime+L"\\ggml-base.dll").c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);need(base!=nullptr,"ggml-base-load");
  decltype(&::ggml_log_set) log_set=nullptr;bind(log_set,base,"ggml_log_set");log_set(quiet_log,nullptr);
  const auto ggml=LoadLibraryExW((runtime+L"\\ggml.dll").c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);need(ggml!=nullptr,"ggml-load");
  decltype(&::ggml_backend_load) load_backend=nullptr;bind(load_backend,ggml,"ggml_backend_load");
  need(load_backend(utf8(runtime+L"\\ggml-cpu-x64.dll").c_str())!=nullptr,"cpu-backend");
  const auto library=LoadLibraryExW((runtime+L"\\llama.dll").c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);need(library!=nullptr,"llama-load");Api api(library);api.llama_log_set(quiet_log,nullptr);
  auto mp=api.llama_model_default_params();mp.n_gpu_layers=0;mp.load_mode=LLAMA_LOAD_MODE_NONE;
  model=api.llama_model_load_from_file(utf8(argv[2]).c_str(),mp);need(model!=nullptr,"model-load");
  auto cp=api.llama_context_default_params();cp.n_ctx=1024;cp.n_batch=1024;cp.n_ubatch=128;cp.n_threads=1;cp.n_threads_batch=1;cp.no_perf=true;
  context=api.llama_init_from_model(model,cp);need(context!=nullptr,"context-load");
  write("GENTLL01",8);write(nonce.data(),nonce.size());
  for(unsigned requests=0;requests<32;++requests){
   uint32_t bytes=0,max_tokens=0;read(&bytes,4);if(!bytes)break;need(bytes<=65536,"request-size");read(&max_tokens,4);need(max_tokens>0&&max_tokens<=64,"request-token-limit");
   std::string prompt(bytes,0);read(prompt.data(),bytes);need(prompt.find('\0')==std::string::npos&&MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,prompt.data(),static_cast<int>(bytes),nullptr,0)>0,"request-utf8");
   const auto vocab=api.llama_model_get_vocab(model);const auto reported=api.llama_tokenize(vocab,prompt.data(),static_cast<int32_t>(bytes),nullptr,0,true,true);
   const auto count=static_cast<int32_t>(gent_model_protocol::required(reported,1024-max_tokens));
   need(count>0&&static_cast<uint32_t>(count)+max_tokens<=1024,"request-context-limit");std::vector<llama_token> tokens(count);
   need(api.llama_tokenize(vocab,prompt.data(),static_cast<int32_t>(bytes),tokens.data(),count,true,true)==count,"tokenize");
   api.llama_memory_clear(api.llama_get_memory(context),true);sampler=api.llama_sampler_init_greedy();need(sampler!=nullptr,"sampler");
   auto batch=api.llama_batch_get_one(tokens.data(),count);llama_token next=0;
   for(uint32_t i=0;i<max_tokens;++i){need(api.llama_decode(context,batch)==0,"decode");next=api.llama_sampler_sample(sampler,context,-1);if(api.llama_vocab_is_eog(vocab,next))break;
    std::vector<char> piece(512);int n=api.llama_token_to_piece(vocab,next,piece.data(),static_cast<int32_t>(piece.size()),0,true);if(n<0){const auto required=gent_model_protocol::required(n,16384);piece.resize(required);n=api.llama_token_to_piece(vocab,next,piece.data(),static_cast<int32_t>(piece.size()),0,true);}gent_model_protocol::piece_size(n,piece.size());if(n)frame(std::string(piece.data(),static_cast<size_t>(n)));batch=api.llama_batch_get_one(&next,1);
   }
   api.llama_sampler_free(sampler);sampler=nullptr;frame("");
  }
  api.llama_free(context);context=nullptr;api.llama_model_free(model);model=nullptr;return 0;
 }catch(const Failure& e){std::fprintf(stderr,"gent-model-worker: %s\n",e.what());return 65;}
 catch(const std::exception&){std::fprintf(stderr,"gent-model-worker: runtime-failure\n");return 65;}
}

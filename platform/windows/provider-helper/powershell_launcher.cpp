#include <windows.h>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>
#include "command_broker_protocol.h"
#include "powershell_launcher_script.h"
namespace {
void require(bool value,const char* label){if(!value)throw std::runtime_error(label);}
struct Handle {
    HANDLE h=nullptr;Handle()=default;explicit Handle(HANDLE value):h(value){}
    ~Handle(){close();}
    void close(){if(h&&h!=INVALID_HANDLE_VALUE)CloseHandle(h);h=nullptr;}
    Handle(const Handle&)=delete;Handle& operator=(const Handle&)=delete;
};
std::wstring final_path(HANDLE h) {
    wchar_t path[32768]{};DWORD n=GetFinalPathNameByHandleW(h,path,32768,FILE_NAME_NORMALIZED|VOLUME_NAME_DOS);
    require(n&&n<32768,"contained-shell-final-path");std::wstring result(path,n);
    while(result.size()>4&&result.back()==L'\\')result.pop_back();return result;
}
std::wstring directory(const std::wstring& path) {
    Handle h(CreateFileW(path.c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
    require(h.h!=INVALID_HANDLE_VALUE,"contained-shell-directory-open");
    BY_HANDLE_FILE_INFORMATION info{};require(GetFileInformationByHandle(h.h,&info)&&
        (info.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY)&&!(info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT),"contained-shell-directory-kind");
    return final_path(h.h);
}
std::wstring environment(const wchar_t* key) {
    wchar_t value[32768]{};DWORD n=GetEnvironmentVariableW(key,value,32768);
    require(n&&n<32768,"contained-shell-workspace-context");return std::wstring(value,n);
}
gent_command::Target target_of_image() {
    wchar_t image[32768]{};DWORD n=GetModuleFileNameW(nullptr,image,32768);require(n&&n<32768,"contained-shell-image");
    const std::wstring path(image,n);const auto name=path.substr(path.find_last_of(L'\\')+1);
    if(contained_ps::equal(name,L"bash.exe"))return gent_command::Target::Bash;
    if(contained_ps::equal(name,L"pwsh.exe"))return gent_command::Target::PowerShell;
    if(contained_ps::equal(name,L"cmd.exe"))return gent_command::Target::Cmd;
    throw std::runtime_error("contained-shell-image-name");
}
std::wstring current_directory() {
    wchar_t cwd[32768]{};DWORD n=GetCurrentDirectoryW(32768,cwd);require(n&&n<32768,"contained-shell-cwd");
    return std::wstring(cwd,n);
}
std::wstring powershell_command(int argc,wchar_t** argv,const std::wstring& cwd) {
    std::vector<std::wstring> args;for(int i=1;i<argc;++i)args.emplace_back(argv[i]);
    args=contained_ps::arguments(std::move(args),directory(environment(L"GENT_POWERSHELL_WORKSPACE")),directory(cwd));
    std::wstring command;for(const auto& arg:args){if(!command.empty())command+=L' ';command+=contained_ps::quote(arg);}
    return command;
}
std::wstring environment_block() {
    auto* block=GetEnvironmentStringsW();require(block!=nullptr,"contained-shell-environment");
    const wchar_t* end=block;while(*end)end+=wcslen(end)+1;
    std::wstring value(block,static_cast<size_t>(end-block)+1);FreeEnvironmentStringsW(block);return value;
}
struct Relay{HANDLE from=nullptr,to=nullptr;bool close_to=false;};
DWORD WINAPI relay_thread(void* value) {
    auto* relay=static_cast<Relay*>(value);char bytes[32768];DWORD got=0;
    while(ReadFile(relay->from,bytes,sizeof(bytes),&got,nullptr)&&got) {
        DWORD offset=0;
        while(offset<got){DWORD wrote=0;if(!WriteFile(relay->to,bytes+offset,got-offset,&wrote,nullptr)||!wrote)goto finished;offset+=wrote;}
    }
finished:
    if(relay->close_to)CloseHandle(relay->to);
    return 0;
}
struct Stream{uint64_t remote=0;Handle child_side;Handle relay_side;Relay relay;Handle thread;bool output=false;};
void prepare_stream(Stream& stream,DWORD id) {
    const HANDLE original=GetStdHandle(id);
    if(!original||original==INVALID_HANDLE_VALUE)return;
    if(GetFileType(original)!=FILE_TYPE_CHAR){stream.remote=reinterpret_cast<uintptr_t>(original);return;}
    HANDLE read_end=nullptr,write_end=nullptr;require(CreatePipe(&read_end,&write_end,nullptr,65536)!=0,"contained-shell-relay-pipe");
    stream.output=id!=STD_INPUT_HANDLE;
    stream.child_side.h=stream.output?write_end:read_end;stream.relay_side.h=stream.output?read_end:write_end;
    stream.remote=reinterpret_cast<uintptr_t>(stream.child_side.h);
    stream.relay.from=stream.output?stream.relay_side.h:original;stream.relay.to=stream.output?original:stream.relay_side.h;
    stream.relay.close_to=!stream.output;if(!stream.output)stream.relay_side.h=nullptr;
    stream.thread.h=CreateThread(nullptr,0,relay_thread,&stream.relay,0,nullptr);require(stream.thread.h!=nullptr,"contained-shell-relay-thread");
}
HANDLE connect_broker() {
    const std::wstring name=std::wstring(L"\\\\.\\pipe\\LOCAL\\")+gent_command::pipe_leaf;
    const auto deadline=GetTickCount64()+10000;
    for(;;) {
        const HANDLE pipe=CreateFileW(name.c_str(),GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr);
        if(pipe!=INVALID_HANDLE_VALUE)return pipe;
        const auto error=GetLastError();
        require((error==ERROR_PIPE_BUSY||error==ERROR_FILE_NOT_FOUND)&&GetTickCount64()<deadline,"contained-shell-broker-unavailable");
        Sleep(20);
    }
}
void read_all(HANDLE pipe,void* data,DWORD size) {
    DWORD done=0;
    while(done<size){DWORD got=0;require(ReadFile(pipe,static_cast<char*>(data)+done,size-done,&got,nullptr)&&got,"contained-shell-broker-closed");done+=got;}
}
void write_all(HANDLE pipe,const void* data,size_t size) {
    size_t done=0;
    while(done<size){DWORD wrote=0;require(WriteFile(pipe,static_cast<const char*>(data)+done,static_cast<DWORD>(size-done),&wrote,nullptr)&&wrote,"contained-shell-broker-write");done+=wrote;}
}
DWORD run_brokered(int argc,wchar_t** argv) {
    gent_command::Request request;request.target=target_of_image();request.cwd=current_directory();
    request.command=request.target==gent_command::Target::PowerShell?powershell_command(argc,argv,request.cwd):gent_command::command_tail(GetCommandLineW());
    require(request.command.size()<=gent_command::max_command_chars,"contained-shell-command-bound");
    request.environment=environment_block();
    Stream streams[3];const DWORD ids[]{STD_INPUT_HANDLE,STD_OUTPUT_HANDLE,STD_ERROR_HANDLE};
    for(size_t i=0;i<3;++i){prepare_stream(streams[i],ids[i]);request.stdio[i]=streams[i].remote;}
    Handle pipe(connect_broker());const auto frame=gent_command::encode(request);write_all(pipe.h,frame.data(),frame.size());
    for(;;) {
        gent_command::ReplyHead head;read_all(pipe.h,&head,sizeof(head));
        switch(static_cast<gent_command::ReplyKind>(head.kind)) {
            case gent_command::ReplyKind::Started:for(auto& stream:streams)stream.child_side.close();break;
            case gent_command::ReplyKind::Exit:
                for(auto& stream:streams)if(stream.output&&stream.thread.h)WaitForSingleObject(stream.thread.h,2000);
                return head.value;
            case gent_command::ReplyKind::Rejected: {
                require(head.value<=4096,"contained-shell-broker-rejected");std::string label(head.value,'\0');
                if(head.value)read_all(pipe.h,label.data(),head.value);
                throw std::runtime_error(label);
            }
            default:throw std::runtime_error("contained-shell-broker-reply");
        }
    }
}
}
int wmain(int argc,wchar_t** argv) {
    try {ExitProcess(run_brokered(argc,argv));}
    catch(const std::exception& error){std::fprintf(stderr,"GENT-CONTAINED-SHELL-ERROR %s\n",error.what());std::fflush(stderr);ExitProcess(126);}
}

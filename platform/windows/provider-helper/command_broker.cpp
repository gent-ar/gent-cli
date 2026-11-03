#include "command_broker.h"
#include "command_broker_protocol.h"
#include "command_environment.h"
#include "contained_job.h"
#include "powershell_launcher_script.h"
#include "provider_capabilities.h"
#include "psec_environment.h"
#include "readonly_authority.h"
#include "win32_paths.h"
#include <sddl.h>
#include <shlobj.h>
#include <array>
#include <atomic>
#include <list>
#include <thread>
namespace {
using gent_psec::require;
constexpr size_t max_active_requests=256;
struct Owned {
    HANDLE h=nullptr;
    Owned()=default;
    explicit Owned(HANDLE value):h(value==INVALID_HANDLE_VALUE?nullptr:value){}
    Owned(const Owned&)=delete;
    Owned& operator=(const Owned&)=delete;
    Owned(Owned&& other) noexcept:h(other.h){other.h=nullptr;}
    Owned& operator=(Owned&& other) noexcept{if(this!=&other){if(h)CloseHandle(h);h=other.h;other.h=nullptr;}return *this;}
    ~Owned(){if(h)CloseHandle(h);}
};
struct Spawned{Owned process,thread,job;};
void transfer(HANDLE pipe,HANDLE stop,void* data,DWORD size,bool write,DWORD timeout) {
    Owned event(CreateEventW(nullptr,TRUE,FALSE,nullptr));require(event.h!=nullptr,"command-pipe-event");
    DWORD done=0;
    while(done<size) {
        OVERLAPPED overlapped{};overlapped.hEvent=event.h;DWORD count=0;auto* cursor=static_cast<unsigned char*>(data)+done;
        BOOL ok=write?WriteFile(pipe,cursor,size-done,&count,&overlapped):ReadFile(pipe,cursor,size-done,&count,&overlapped);
        if(!ok&&GetLastError()==ERROR_IO_PENDING) {
            HANDLE waits[]{event.h,stop};
            if(WaitForMultipleObjects(2,waits,FALSE,timeout)!=WAIT_OBJECT_0) {
                CancelIoEx(pipe,&overlapped);GetOverlappedResult(pipe,&overlapped,&count,TRUE);
                throw std::runtime_error("command-pipe-io-cancelled");
            }
            ok=GetOverlappedResult(pipe,&overlapped,&count,FALSE);
        }
        require(ok&&count,"command-pipe-io");done+=count;
    }
}
std::wstring known_folder(REFKNOWNFOLDERID id) {
    PWSTR value=nullptr;std::wstring result;
    if(SUCCEEDED(SHGetKnownFolderPath(id,0,nullptr,&value))&&value)result=value;
    CoTaskMemFree(value);return result;
}
std::wstring windows_directory() {
    std::wstring value(32768,L'\0');const auto size=GetWindowsDirectoryW(value.data(),static_cast<UINT>(value.size()));
    require(size&&size<value.size(),"command-windows-directory");value.resize(size);return value;
}
void private_directory(const std::wstring& path) {
    const auto io=gent_windows_paths::io_path(path);
    require(CreateDirectoryW(io.c_str(),nullptr)||GetLastError()==ERROR_ALREADY_EXISTS,"command-directory-create");
    const auto attributes=GetFileAttributesW(io.c_str());
    require(attributes!=INVALID_FILE_ATTRIBUTES&&(attributes&FILE_ATTRIBUTE_DIRECTORY)&&!(attributes&FILE_ATTRIBUTE_REPARSE_POINT),"command-directory-kind");
}
std::wstring parent_of(const std::wstring& path){return path.substr(0,path.find_last_of(L'\\'));}
}
struct CommandBroker::State {
    struct Worker{std::thread thread;std::atomic<bool> done{false};};
    CommandBrokerConfig config;
    gent_psec::Api api;
    std::unique_ptr<gent_psec::Environment> environment;
    gent_command::EnvironmentPolicy environment_policy;
    std::vector<unsigned char> provider_sid;
    std::vector<std::wstring> command_roots,authority_read,command_write;
    std::wstring pipe_name,cmd_image;
    PSECURITY_DESCRIPTOR pipe_security=nullptr;
    Owned stop_event,command_root;
    std::thread accept_thread;
    std::list<Worker> workers;
    std::atomic<bool> unsettled{false},stopped{false};
    explicit State(CommandBrokerConfig value):config(std::move(value)),stop_event(CreateEventW(nullptr,TRUE,FALSE,nullptr)) {
        require(stop_event.h!=nullptr,"command-stop-event");
        command_root.h=CreateJobObjectW(nullptr,nullptr);require(command_root.h!=nullptr,"command-root-job");
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
        require(QueryInformationJobObject(config.run_job,JobObjectExtendedLimitInformation,&limits,sizeof(limits),nullptr)!=0,"command-root-job-query");
        require(SetInformationJobObject(command_root.h,JobObjectExtendedLimitInformation,&limits,sizeof(limits))!=0,"command-root-job-limit");
        provider_sid=token_appcontainer_sid(config.provider_token);
        const auto windows=windows_directory();cmd_image=windows+L"\\System32\\cmd.exe";
        const auto policy=build_policy(windows);
        environment=std::make_unique<gent_psec::Environment>(api,gent_psec::Spec(policy));
        bootstrap(windows);
        start_pipe();
    }
    ~State(){if(pipe_security)LocalFree(pipe_security);}
    gent_psec::Policy build_policy(const std::wstring& windows) {
        const auto& private_root=config.state_root;
        const auto temp=config.ordinary_state+L"\\tmp",home=config.ordinary_state+L"\\home";
        private_directory(temp+L"\\command-localappdata");private_directory(home+L"\\appdata");
        environment_policy.shim_directory=config.shim_directory;environment_policy.windows_directory=windows;
        environment_policy.real_shell_directories={parent_of(config.bash),parent_of(config.powershell)};
        environment_policy.bash_image=config.bash;environment_policy.powershell_image=config.powershell;
        environment_policy.git_ceiling=L";"+parent_of(config.ordinary_workspace);
        for(auto& character:environment_policy.git_ceiling)if(character==L'\\')character=L'/';
        const auto program_files=known_folder(FOLDERID_ProgramFiles);
        for(const auto& tool:{parent_of(config.bash),program_files+L"\\Git\\cmd",program_files+L"\\Git\\mingw64\\bin",program_files+L"\\nodejs",config.node_directory}) {
            const auto attributes=tool.empty()?INVALID_FILE_ATTRIBUTES:GetFileAttributesW(tool.c_str());
            if(attributes!=INVALID_FILE_ATTRIBUTES&&(attributes&FILE_ATTRIBUTE_DIRECTORY))environment_policy.tool_directories.push_back(tool);
        }
        environment_policy.bearer=config.bearer;environment_policy.gateway_origin=config.gateway_origin;
        environment_policy.temp_root=temp;environment_policy.appdata=home+L"\\appdata";
        environment_policy.localappdata=temp+L"\\command-localappdata";
        gent_psec::Policy policy;policy.network=gent_psec::Network::DirectInternet;
        const auto private_normalized=gent_psec::normalized_path(private_root);
        for(const auto& granted:config.write)if(_wcsicmp(gent_psec::normalized_path(granted).c_str(),private_normalized.c_str())!=0)policy.write.push_back(granted);
        policy.write.push_back(private_root+L"\\tmp");policy.write.push_back(private_root+L"\\home");
        policy.read=config.read;policy.read.push_back(config.workspace);policy.read.push_back(private_root);
        if(!config.node_directory.empty())policy.read.push_back(parent_of(config.node_directory));
        authority_read=policy.read;command_write=policy.write;
        const auto claurst=private_root+L"\\home\\.claurst";
        if(GetFileAttributesW(gent_windows_paths::io_path(claurst).c_str())!=INVALID_FILE_ATTRIBUTES)policy.nested_read_only.push_back(claurst);
        policy.read.push_back(windows);
        for(const auto& system_root:{known_folder(FOLDERID_ProgramFiles),known_folder(FOLDERID_ProgramFilesX86)})
            if(!system_root.empty()&&GetFileAttributesW(system_root.c_str())!=INVALID_FILE_ATTRIBUTES)policy.read.push_back(system_root);
        for(const auto* list:{&policy.read,&policy.write})for(const auto& granted:*list)command_roots.push_back(gent_psec::normalized_path(granted));
        return policy;
    }
    Spawned spawn(const std::wstring& image,std::wstring line,std::vector<wchar_t>& block,const std::wstring& cwd,HANDLE in,HANDLE out,HANDLE err,bool verify_authority) {
        require(line.size()<32760,"command-line-bound");
        SIZE_T attribute_size=0;InitializeProcThreadAttributeList(nullptr,2,0,&attribute_size);std::vector<unsigned char> storage(attribute_size);
        auto attributes=reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
        require(InitializeProcThreadAttributeList(attributes,2,0,&attribute_size)!=0,"command-attributes");
        struct Cleanup{LPPROC_THREAD_ATTRIBUTE_LIST list;~Cleanup(){DeleteProcThreadAttributeList(list);}} cleanup{attributes};
        require(UpdateProcThreadAttribute(attributes,0,gent_psec::security_environment_attribute,&environment->value,sizeof(environment->value),nullptr,nullptr)!=0,"command-attach-psec");
        HANDLE inherited[]{in,out,err};
        require(UpdateProcThreadAttribute(attributes,0,PROC_THREAD_ATTRIBUTE_HANDLE_LIST,inherited,sizeof(inherited),nullptr,nullptr)!=0,"command-stdio-handles");
        STARTUPINFOEXW startup{};startup.StartupInfo.cb=sizeof(startup);startup.StartupInfo.dwFlags=STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput=in;startup.StartupInfo.hStdOutput=out;startup.StartupInfo.hStdError=err;startup.lpAttributeList=attributes;
        PROCESS_INFORMATION created{};
        require(CreateProcessW(image.c_str(),line.data(),nullptr,nullptr,TRUE,CREATE_SUSPENDED|CREATE_NO_WINDOW|CREATE_UNICODE_ENVIRONMENT|EXTENDED_STARTUPINFO_PRESENT,block.data(),cwd.c_str(),&startup.StartupInfo,&created)!=0,"command-create");
        Spawned result;result.process.h=created.hProcess;result.thread.h=created.hThread;
        try {
            bind_suspended_to_job(result.process.h,result.thread.h,command_root.h);
            result.job.h=CreateJobObjectW(nullptr,nullptr);require(result.job.h!=nullptr,"command-job");
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            require(SetInformationJobObject(result.job.h,JobObjectExtendedLimitInformation,&limits,sizeof(limits))!=0,"command-job-limit");
            require(AssignProcessToJobObject(result.job.h,result.process.h)!=0,"command-job-assign");
            HANDLE raw=nullptr;require(OpenProcessToken(result.process.h,TOKEN_QUERY|TOKEN_DUPLICATE,&raw)!=0,"command-token");Owned token(raw);
            require(token_appcontainer_sid(token.h)!=provider_sid,"command-token-shares-provider-container");
            validate_command_capabilities(token.h);validate_low_integrity(token.h);
            if(verify_authority)validate_readonly_authority(token.h,authority_read,command_write);
            require(ResumeThread(result.thread.h)!=static_cast<DWORD>(-1),"command-resume");
        } catch(...) {TerminateProcess(result.process.h,126);WaitForSingleObject(result.process.h,5000);throw;}
        return result;
    }
    Owned null_device() {
        SECURITY_ATTRIBUTES inheritable{sizeof(inheritable),nullptr,TRUE};
        Owned device(CreateFileW(L"NUL",GENERIC_READ|GENERIC_WRITE,FILE_SHARE_READ|FILE_SHARE_WRITE,&inheritable,OPEN_EXISTING,0,nullptr));
        require(device.h!=nullptr,"command-null-device");return device;
    }
    void bootstrap(const std::wstring& windows) {
        const auto base=environment_policy.localappdata;
        std::vector<std::wstring> entries{L"LOCALAPPDATA="+base,L"PATH="+windows+L"\\System32;"+windows,L"SystemDrive="+windows.substr(0,2),L"SystemRoot="+windows,L"TEMP="+environment_policy.temp_root,L"windir="+windows};
        std::vector<wchar_t> block;for(const auto& entry:entries){block.insert(block.end(),entry.begin(),entry.end());block.push_back(0);}block.push_back(0);
        auto in=null_device(),out=null_device(),err=null_device();
        auto process=spawn(config.helper_image,contained_ps::quote(config.helper_image)+L" bootstrap-temp "+contained_ps::quote(base),block,environment_policy.temp_root,in.h,out.h,err.h,true);
        require(WaitForSingleObject(process.process.h,10000)==WAIT_OBJECT_0,"command-bootstrap-timeout");
        DWORD code=1;require(GetExitCodeProcess(process.process.h,&code)&&code==0,"command-bootstrap-failed");
        require(wait_owned_job_empty(process.job.h,5000),"command-bootstrap-job");
    }
    HANDLE new_instance(DWORD flags) {
        SECURITY_ATTRIBUTES attributes{sizeof(attributes),pipe_security,FALSE};
        const auto pipe=CreateNamedPipeW(pipe_name.c_str(),PIPE_ACCESS_DUPLEX|FILE_FLAG_OVERLAPPED|flags,PIPE_TYPE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,PIPE_UNLIMITED_INSTANCES,65536,65536,0,&attributes);
        require(pipe!=INVALID_HANDLE_VALUE,"command-pipe-create");return pipe;
    }
    void start_pipe() {
        LPWSTR sid_text=nullptr;require(ConvertSidToStringSidW(provider_sid.data(),&sid_text)!=0,"command-sid-text");
        const std::wstring sid(sid_text);LocalFree(sid_text);DWORD session=0;
        require(ProcessIdToSessionId(GetCurrentProcessId(),&session)!=0,"command-session");
        pipe_name=L"\\\\.\\pipe\\Sessions\\"+std::to_wstring(session)+L"\\AppContainerNamedObjects\\"+sid+L"\\"+gent_command::pipe_leaf;
        const std::wstring sddl=L"D:P(A;;GA;;;OW)(A;;GA;;;"+sid+L")S:(ML;;NW;;;LW)";
        require(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(),SDDL_REVISION_1,&pipe_security,nullptr)!=0,"command-pipe-security");
        Owned first(new_instance(FILE_FLAG_FIRST_PIPE_INSTANCE));
        accept_thread=std::thread([this,pipe=std::move(first)]() mutable {accept(std::move(pipe));});
    }
    void accept(Owned listening) noexcept {
        try {
            Owned event(CreateEventW(nullptr,TRUE,FALSE,nullptr));require(event.h!=nullptr,"command-accept-event");
            for(;;) {
                OVERLAPPED overlapped{};overlapped.hEvent=event.h;ResetEvent(event.h);
                if(!ConnectNamedPipe(listening.h,&overlapped)) {
                    const auto error=GetLastError();
                    if(error==ERROR_IO_PENDING) {
                        HANDLE waits[]{event.h,stop_event.h};
                        if(WaitForMultipleObjects(2,waits,FALSE,INFINITE)!=WAIT_OBJECT_0){CancelIoEx(listening.h,&overlapped);DWORD ignored=0;GetOverlappedResult(listening.h,&overlapped,&ignored,TRUE);return;}
                    } else if(error!=ERROR_PIPE_CONNECTED) throw std::runtime_error("command-pipe-connect");
                }
                Owned next(new_instance(0));
                workers.remove_if([](Worker& worker){if(!worker.done)return false;worker.thread.join();return true;});
                auto& worker=workers.emplace_back();
                worker.thread=std::thread([this,&worker,pipe=std::move(listening),over=workers.size()>max_active_requests]() mutable {
                    serve(std::move(pipe),over);worker.done=true;});
                listening=std::move(next);
            }
        } catch(...) {unsettled=true;}
    }
    void reply(HANDLE pipe,gent_command::ReplyKind kind,uint32_t value) {
        gent_command::ReplyHead head;head.kind=static_cast<uint32_t>(kind);head.value=value;
        transfer(pipe,stop_event.h,&head,sizeof(head),true,5000);
    }
    void serve(Owned pipe,bool over_capacity) noexcept {
        try {
            require(!over_capacity,"command-capacity");
            handle(pipe.h);
        } catch(const std::exception& error) {
            try {
                std::string label(error.what());
                reply(pipe.h,gent_command::ReplyKind::Rejected,static_cast<uint32_t>(label.size()));
                transfer(pipe.h,stop_event.h,label.data(),static_cast<DWORD>(label.size()),true,5000);
                FlushFileBuffers(pipe.h);
            } catch(...) {}
        } catch(...) {}
    }
    const std::wstring& image(gent_command::Target target) const {
        switch(target) {
            case gent_command::Target::Bash:return config.bash;
            case gent_command::Target::PowerShell:return config.powershell;
            case gent_command::Target::Cmd:return cmd_image;
        }
        throw std::runtime_error("command-target");
    }
    Owned duplicate_stream(HANDLE client,uint64_t value) {
        if(!value)return null_device();
        HANDLE copy=nullptr;
        require(DuplicateHandle(client,reinterpret_cast<HANDLE>(static_cast<uintptr_t>(value)),GetCurrentProcess(),&copy,0,TRUE,DUPLICATE_SAME_ACCESS)!=0,"command-stdio-duplicate");
        Owned owned(copy);const auto kind=GetFileType(owned.h);
        if(kind!=FILE_TYPE_DISK&&kind!=FILE_TYPE_PIPE)throw std::runtime_error("command-stdio-kind");
        return owned;
    }
    void handle(HANDLE pipe) {
        ULONG pid=0;require(GetNamedPipeClientProcessId(pipe,&pid)&&pid,"command-client-pid");
        Owned client(OpenProcess(PROCESS_DUP_HANDLE|SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid));require(client.h!=nullptr,"command-client-open");
        HANDLE raw=nullptr;require(OpenProcessToken(client.h,TOKEN_QUERY,&raw)!=0,"command-client-token");Owned token(raw);
        if(token_appcontainer_sid(token.h)!=provider_sid)throw std::runtime_error("command-client-container");
        BOOL member=FALSE;require(IsProcessInJob(client.h,config.run_job,&member)&&member,"command-client-job");
        uint32_t size=0;transfer(pipe,stop_event.h,&size,sizeof(size),false,10000);
        if(size>gent_command::max_frame_bytes)throw std::runtime_error("command-request-size");
        std::vector<unsigned char> body(size);if(size)transfer(pipe,stop_event.h,body.data(),size,false,10000);
        auto request=gent_command::decode(body);
        const auto& executable=image(request.target);
        if(executable.empty())throw std::runtime_error("command-target-unavailable");
        if(!gent_command::command_directory_allowed(request.cwd,command_roots))throw std::runtime_error("command-cwd");
        auto block=gent_command::command_environment(request.environment,environment_policy);
        Spawned command;
        {
            auto in=duplicate_stream(client.h,request.stdio[0]),out=duplicate_stream(client.h,request.stdio[1]),err=duplicate_stream(client.h,request.stdio[2]);
            auto line=contained_ps::quote(executable);if(!request.command.empty())line+=L" "+request.command;
            command=spawn(executable,std::move(line),block,request.cwd,in.h,out.h,err.h,false);
        }
        reply(pipe,gent_command::ReplyKind::Started,0);
        HANDLE waits[]{command.process.h,client.h,stop_event.h};
        const auto signaled=WaitForMultipleObjects(3,waits,FALSE,INFINITE);
        if(signaled==WAIT_OBJECT_0) {
            DWORD code=126;require(GetExitCodeProcess(command.process.h,&code)!=0,"command-exit");
            try {reply(pipe,gent_command::ReplyKind::Exit,code);FlushFileBuffers(pipe);} catch(...) {}
            while(!wait_owned_job_empty(command.job.h,0)&&WaitForSingleObject(stop_event.h,200)==WAIT_TIMEOUT){}
        }
        if(!TerminateJobObject(command.job.h,126)||!wait_owned_job_empty(command.job.h,5000))unsettled=true;
    }
    std::string accounting() {
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION counters{};
        require(QueryInformationJobObject(command_root.h,JobObjectBasicAccountingInformation,&counters,sizeof(counters),nullptr)!=0,"command-root-job-accounting");
        return "command_job_active="+std::to_string(counters.ActiveProcesses)+" command_job_total="+std::to_string(counters.TotalProcesses);
    }
    bool stop() noexcept {
        if(!stopped.exchange(true)) {
            SetEvent(stop_event.h);
            if(accept_thread.joinable())accept_thread.join();
            for(auto& worker:workers)if(worker.thread.joinable())worker.thread.join();
            if(!TerminateJobObject(command_root.h,126)||!wait_owned_job_empty(command_root.h,5000))unsettled=true;
        }
        return !unsettled;
    }
};
CommandBroker::CommandBroker(CommandBrokerConfig config):state(std::make_unique<State>(std::move(config))){}
CommandBroker::~CommandBroker(){if(state)state->stop();}
bool CommandBroker::stop() noexcept {return state->stop();}
std::string CommandBroker::accounting() {return state->accounting();}

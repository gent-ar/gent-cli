#pragma once
#include <atomic>
#include <cwchar>
// Host-side credential transport only. This is NOT the user-run PSEC route.
// Included after the existing Handle/check/hash/quote/directory_lock helpers.
struct CredentialProbeRead {
    std::vector<char> bytes;
    bool any=false, invalid=false;
};
static void credential_probe_read(HANDLE pipe, CredentialProbeRead& result, bool capture) noexcept {
    std::array<char,16384> buffer{};
    for (;;) {
        DWORD count=0;
        if (!ReadFile(pipe,buffer.data(),static_cast<DWORD>(buffer.size()),&count,nullptr)) {
            if (GetLastError()!=ERROR_BROKEN_PIPE) result.invalid=true;
            return;
        }
        if (!count) return;
        result.any=true;
        if (capture && !result.invalid) {
            if (count>65536-result.bytes.size()) { result.invalid=true;result.bytes.clear(); }
            else result.bytes.insert(result.bytes.end(),buffer.data(),buffer.data()+count);
        }
    }
}
static bool credential_probe_drain(std::thread& reader) noexcept {
    auto handle=static_cast<HANDLE>(reader.native_handle());
    if (WaitForSingleObject(handle,2000)!=WAIT_OBJECT_0) {
        CancelSynchronousIo(handle);
        if (WaitForSingleObject(handle,2000)!=WAIT_OBJECT_0) return false;
    }
    reader.join();return true;
}
static int run_credential_probe(int argc,wchar_t** argv) {
    check(argc>=8 && argc<=128,"credential-probe-arguments");
    check(std::wstring(argv[6])==L"--" && digest_string(argv[3]),"credential-probe-binding");
    std::wstring duration=argv[5];
    check(!duration.empty() && duration.size()<=5 && std::all_of(duration.begin(),duration.end(),[](wchar_t c){return c>=L'0'&&c<=L'9';}),"credential-probe-timeout");
    const auto timeout=std::stoul(duration);check(timeout>0 && timeout<=45000,"credential-probe-timeout-bound");
    auto image_directory=directory_lock(argv[2]);auto working_directory=directory_lock(argv[4]);
    Handle image(CreateFileW(argv[2],GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
    check(image.h!=INVALID_HANDLE_VALUE,"credential-probe-image-lock");
    BY_HANDLE_FILE_INFORMATION info{};check(GetFileInformationByHandle(image.h,&info)!=FALSE,"credential-probe-image-info");
    check(!(info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)) && hash(image.h)==argv[3],"credential-probe-image-digest");
    std::wstring command=quote(argv[2]);
    for (int i=7;i<argc;++i) {
        std::wstring argument=argv[i];check(argument.size()<=8192,"credential-probe-argument-bound");
        // Match normal Windows argv encoding: simple flags remain bare;
        // empty/whitespace/quote arguments use the existing CRT escaping.
        command+=L" "+(argument.empty()||argument.find_first_of(L" \t\n\v\"")!=std::wstring::npos?quote(argument):argument);
    }
    check(command.size()<28000,"credential-probe-command-bound");
    SECURITY_ATTRIBUTES security{sizeof(security),nullptr,TRUE};HANDLE out_r=nullptr,out_w=nullptr,err_r=nullptr,err_w=nullptr;
    check(CreatePipe(&out_r,&out_w,&security,0)!=FALSE,"credential-probe-stdout");Handle output_read(out_r),output_write(out_w);
    check(CreatePipe(&err_r,&err_w,&security,0)!=FALSE,"credential-probe-stderr");Handle error_read(err_r),error_write(err_w);
    check(SetHandleInformation(output_read.h,HANDLE_FLAG_INHERIT,0)!=FALSE && SetHandleInformation(error_read.h,HANDLE_FLAG_INHERIT,0)!=FALSE,"credential-probe-parent-pipes");
    Handle input(CreateFileW(L"NUL",GENERIC_READ,FILE_SHARE_READ|FILE_SHARE_WRITE,&security,OPEN_EXISTING,0,nullptr));
    check(input.h!=INVALID_HANDLE_VALUE,"credential-probe-input");
    Handle job(CreateJobObjectW(nullptr,nullptr));check(job.h!=nullptr,"credential-probe-job");
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    check(SetInformationJobObject(job.h,JobObjectExtendedLimitInformation,&limits,sizeof(limits))!=FALSE,"credential-probe-job-limits");
    OwnedJobLifetime lifetime{job.h};
    // The daemon owns the sole control writer. SDK stdin is a distinct NUL
    // handle, so a surviving SDK descendant cannot hide daemon death.
    const auto control_input=GetStdHandle(STD_INPUT_HANDLE);
    check(GetFileType(control_input)==FILE_TYPE_PIPE,"credential-probe-control");
    std::atomic<bool> cancelled{false};
    std::thread control([&]{char byte=0;DWORD count=0;ReadFile(control_input,&byte,1,&count,nullptr);cancelled.store(true);TerminateJobObject(job.h,126);});
    SIZE_T size=0;InitializeProcThreadAttributeList(nullptr,1,0,&size);std::vector<unsigned char> storage(size);
    auto attributes=reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
    check(InitializeProcThreadAttributeList(attributes,1,0,&size)!=FALSE,"credential-probe-attributes");
    struct AttributeCleanup { LPPROC_THREAD_ATTRIBUTE_LIST value;~AttributeCleanup(){DeleteProcThreadAttributeList(value);} } cleanup{attributes};
    HANDLE inherited[]={input.h,output_write.h,error_write.h};
    check(UpdateProcThreadAttribute(attributes,0,PROC_THREAD_ATTRIBUTE_HANDLE_LIST,inherited,sizeof(inherited),nullptr,nullptr)!=FALSE,"credential-probe-handle-list");
    STARTUPINFOEXW startup{};startup.StartupInfo.cb=sizeof(startup);startup.StartupInfo.dwFlags=STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput=input.h;startup.StartupInfo.hStdOutput=output_write.h;startup.StartupInfo.hStdError=error_write.h;startup.lpAttributeList=attributes;
    PROCESS_INFORMATION process{};
    check(!cancelled.load(),"credential-probe-parent-gone");
    check(CreateProcessW(argv[2],command.data(),nullptr,nullptr,TRUE,CREATE_SUSPENDED|CREATE_NO_WINDOW|EXTENDED_STARTUPINFO_PRESENT,nullptr,argv[4],&startup.StartupInfo,&process)!=FALSE,"credential-probe-create");
    Handle child(process.hProcess),thread(process.hThread);
    // If assignment itself fails the suspended leader is still ours, but not
    // yet in the Job: kill it explicitly before reporting Unknown.
    try { bind_suspended_to_job(child.h,thread.h,job.h);check(!cancelled.load()&&ResumeThread(thread.h)==1,"credential-probe-resume"); }
    catch (...) {TerminateProcess(child.h,126);WaitForSingleObject(child.h,2000);throw;}
    output_write.close();error_write.close();input.close();
    CredentialProbeRead output,error;
    std::thread stdout_reader([&]{credential_probe_read(output_read.h,output,true);});
    std::thread stderr_reader([&]{credential_probe_read(error_read.h,error,false);});
    const auto wait=WaitForSingleObject(child.h,static_cast<DWORD>(timeout));DWORD code=126;
    const bool leader_known=wait==WAIT_OBJECT_0 && GetExitCodeProcess(child.h,&code)!=FALSE;
    const bool stopped=TerminateJobObject(job.h,126)!=FALSE && wait_owned_job_empty(job.h,2000);
    if (!stopped) ExitProcess(126); // KILL_ON_CLOSE; no Known frame, no joins.
    if (!credential_probe_drain(stdout_reader) || !credential_probe_drain(stderr_reader)) ExitProcess(126);
    CancelSynchronousIo(static_cast<HANDLE>(control.native_handle()));
    if (!credential_probe_drain(control)) ExitProcess(126);
    uint32_t flags=(leader_known&&code==0?1u:0u)|(!error.any?2u:0u)|(!output.invalid&&!error.invalid?4u:0u);
    uint32_t header[3]={flags,code,static_cast<uint32_t>(output.bytes.size())};
    const char magic[8]={'G','E','N','T','P','R','B','2'};
    auto write=[](const void* bytes,DWORD count){DWORD written=0;return WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),bytes,count,&written,nullptr)!=FALSE&&written==count;};
    check(write(magic,static_cast<DWORD>(sizeof(magic)))&&write(header,static_cast<DWORD>(sizeof(header)))&&(output.bytes.empty()||write(output.bytes.data(),static_cast<DWORD>(output.bytes.size()))),"credential-probe-result");
    return 0; // Frame is emitted only after exact whole-Job0 and reader0.
}

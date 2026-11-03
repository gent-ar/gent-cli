#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <cstdio>
#include <string>
#include <vector>
static void attempt(const wchar_t* target, int which, bool pipes) {
    HANDLE file=CreateFileW(target,GENERIC_READ|GENERIC_EXECUTE,FILE_SHARE_READ,nullptr,OPEN_EXISTING,0,nullptr);
    DWORD open_error=file==INVALID_HANDLE_VALUE?GetLastError():0;
    if(file!=INVALID_HANDLE_VALUE)CloseHandle(file);
    HANDLE rd[3]{},wr[3]{}; DWORD pipe_error=0;
    SECURITY_ATTRIBUTES sa{sizeof(sa),nullptr,TRUE};
    if(pipes)for(int i=0;i<3;++i) {
        if(!CreatePipe(&rd[i],&wr[i],&sa,0)){pipe_error=GetLastError();break;}
        SetHandleInformation(i==0?wr[i]:rd[i],HANDLE_FLAG_INHERIT,0);
    }
    STARTUPINFOW si{};si.cb=sizeof(si);
    if(pipes){si.dwFlags=STARTF_USESTDHANDLES;si.hStdInput=rd[0];si.hStdOutput=wr[1];si.hStdError=wr[2];}
    PROCESS_INFORMATION pi{};
    std::wstring command=L"\""+std::wstring(target)+L"\" --version";
    BOOL ok=FALSE;DWORD error=pipe_error;
    if(!pipe_error){ok=CreateProcessW(target,command.data(),nullptr,nullptr,pipes,CREATE_SUSPENDED|CREATE_NO_WINDOW,nullptr,nullptr,&si,&pi);error=ok?0:GetLastError();}
    DWORD terminated=0;
    if(ok){terminated=TerminateProcess(pi.hProcess,0)&&WaitForSingleObject(pi.hProcess,5000)==WAIT_OBJECT_0;CloseHandle(pi.hThread);CloseHandle(pi.hProcess);}
    for(int i=0;i<3;++i){if(rd[i])CloseHandle(rd[i]);if(wr[i])CloseHandle(wr[i]);}
    std::printf("CHILDSPAWN target=%d pipes=%d open=%lu pipe=%lu spawn=%lu terminated=%lu\n",which,pipes,open_error,pipe_error,error,terminated);
    std::fflush(stdout);
}
int wmain(int argc,wchar_t** argv){
    if(argc!=3)return 2;
    for(int i=1;i<3;++i){attempt(argv[i],i,false);attempt(argv[i],i,true);}
    return 0;
}

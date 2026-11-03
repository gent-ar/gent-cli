#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <utility>
#include <sstream>
#include <string>
#include <vector>
namespace {
int connect_error(const std::string& ip,uint16_t port) {
    SOCKET s=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);if(s==INVALID_SOCKET)return WSAGetLastError();
    u_long nonblock=1;ioctlsocket(s,FIONBIO,&nonblock);
    sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(port);if(InetPtonA(AF_INET,ip.c_str(),&a.sin_addr)!=1){closesocket(s);return -2;}
    int r=connect(s,reinterpret_cast<sockaddr*>(&a),sizeof(a));int e=r?WSAGetLastError():0;
    if(e==WSAEWOULDBLOCK) {
        fd_set w,x;FD_ZERO(&w);FD_ZERO(&x);FD_SET(s,&w);FD_SET(s,&x);timeval t{3,0};int ready=select(0,nullptr,&w,&x,&t);
        if(ready>0){int len=sizeof(e);getsockopt(s,SOL_SOCKET,SO_ERROR,reinterpret_cast<char*>(&e),&len);}else e=ready==0?WSAETIMEDOUT:WSAGetLastError();
    }
    closesocket(s);return e;
}
int resolve_error(const char* name){addrinfo* out=nullptr;int e=getaddrinfo(name,"443",nullptr,&out);if(out)freeaddrinfo(out);return e;}
std::string narrow(const std::wstring& w){if(w.empty())return{};int n=WideCharToMultiByte(CP_UTF8,0,w.data(),static_cast<int>(w.size()),nullptr,0,nullptr,nullptr);std::string s(n,'\0');WideCharToMultiByte(CP_UTF8,0,w.data(),static_cast<int>(w.size()),s.data(),n,nullptr,nullptr);return s;}
std::wstring wide(const std::string& s){int n=MultiByteToWideChar(CP_UTF8,0,s.data(),static_cast<int>(s.size()),nullptr,0);std::wstring w(n,L'\0');MultiByteToWideChar(CP_UTF8,0,s.data(),static_cast<int>(s.size()),w.data(),n);return w;}
std::string getenv_utf8(const wchar_t* key){wchar_t v[32768]{};DWORD n=GetEnvironmentVariableW(key,v,32768);return narrow(std::wstring(v,n));}
bool write_text(const std::string& path,const std::string& text){std::ofstream f(wide(path),std::ios::binary);f<<text;return f.good();}
std::string read_text(const std::string& path){std::ifstream f(wide(path),std::ios::binary);if(!f)return"<denied>";std::stringstream s;s<<f.rdbuf();return s.str();}
bool can_write(const std::string& path){const auto p=wide(path);HANDLE h=CreateFileW(p.c_str(),GENERIC_WRITE,0,nullptr,CREATE_ALWAYS,0,nullptr);if(h==INVALID_HANDLE_VALUE)return false;CloseHandle(h);DeleteFileW(p.c_str());return true;}
bool can_read(const std::string& path){HANDLE h=CreateFileW(wide(path).c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,0,nullptr);if(h==INVALID_HANDLE_VALUE)return false;CloseHandle(h);return true;}
void emit(const std::string& line){const auto text=line+"\n";DWORD n=0;WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),text.data(),static_cast<DWORD>(text.size()),&n,nullptr);}
int gateway_http(uint16_t port,const std::string& token,std::string* status) {
    SOCKET s=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);DWORD timeout=8000;setsockopt(s,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<char*>(&timeout),sizeof(timeout));
    sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(port);a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
    if(connect(s,reinterpret_cast<sockaddr*>(&a),sizeof(a))){const int e=WSAGetLastError();closesocket(s);return e;}
    const auto request="GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:"+std::to_string(port)+"\r\nAuthorization: Bearer "+token+"\r\nConnection: close\r\n\r\n";
    send(s,request.data(),static_cast<int>(request.size()),0);std::string reply;char bytes[1024];int n=0;
    while((n=recv(s,bytes,sizeof(bytes),0))>0)reply.append(bytes,n);closesocket(s);*status=reply.substr(0,reply.find("\r\n"));return 0;
}
uint16_t origin_port(const std::string& origin){const auto c=origin.find_last_of(':');return c==std::string::npos?0:static_cast<uint16_t>(std::stoul(origin.substr(c+1)));}
struct Child{PROCESS_INFORMATION pi{};HANDLE out=nullptr;};
std::wstring argv_quote(const std::wstring& value) {
    std::wstring result=L"\"";size_t slashes=0;
    for(wchar_t c:value){if(c==L'\\'){++slashes;continue;}result.append(c==L'"'?slashes*2+1:slashes,L'\\');slashes=0;result+=c;}
    result.append(slashes*2,L'\\');return result+L'"';
}
Child spawn_shim(const std::wstring& image,std::wstring args,const std::wstring& cwd) {
    SECURITY_ATTRIBUTES sa{sizeof(sa),nullptr,TRUE};HANDLE r=nullptr,w=nullptr;CreatePipe(&r,&w,&sa,65536);SetHandleInformation(r,HANDLE_FLAG_INHERIT,0);
    HANDLE nul=CreateFileW(L"NUL",GENERIC_READ,FILE_SHARE_READ|FILE_SHARE_WRITE,&sa,OPEN_EXISTING,0,nullptr);
    STARTUPINFOW si{};si.cb=sizeof(si);si.dwFlags=STARTF_USESTDHANDLES;si.hStdOutput=w;si.hStdError=w;si.hStdInput=nul;
    std::wstring line=argv_quote(image)+L" "+args;Child c;
    if(!CreateProcessW(image.c_str(),line.data(),nullptr,nullptr,TRUE,CREATE_NO_WINDOW,nullptr,cwd.c_str(),&si,&c.pi))emit("spawn-error="+std::to_string(GetLastError()));
    CloseHandle(w);CloseHandle(nul);c.out=r;return c;
}
std::string drain(HANDLE from){std::string text;char b[4096];DWORD n=0;while(ReadFile(from,b,sizeof(b),&n,nullptr)&&n)text.append(b,n);CloseHandle(from);return text;}
int wait_exit(Child& c){WaitForSingleObject(c.pi.hProcess,120000);DWORD code=999;GetExitCodeProcess(c.pi.hProcess,&code);CloseHandle(c.pi.hProcess);CloseHandle(c.pi.hThread);return static_cast<int>(code);}
bool wait_for_file(const std::string& path,DWORD ms){const auto end=GetTickCount64()+ms;while(GetTickCount64()<end){if(GetFileAttributesW(wide(path).c_str())!=INVALID_FILE_ATTRIBUTES)return true;Sleep(50);}return false;}
std::string posix(std::string path){for(auto& c:path)if(c=='\\')c='/';return path;}
std::string single(const std::string& value){return "'"+value+"'";}
std::string conpty_run(const std::wstring& image,const std::wstring& args,const std::wstring& cwd,int* exit_code) {
    HANDLE in_r=nullptr,in_w=nullptr,out_r=nullptr,out_w=nullptr;CreatePipe(&in_r,&in_w,nullptr,0);CreatePipe(&out_r,&out_w,nullptr,0);
    HPCON console=nullptr;const HRESULT created=CreatePseudoConsole(COORD{120,30},in_r,out_w,0,&console);
    if(FAILED(created)){*exit_code=-1;return "conpty-create-failed:"+std::to_string(static_cast<unsigned long>(created));}
    SIZE_T size=0;InitializeProcThreadAttributeList(nullptr,1,0,&size);std::vector<unsigned char> storage(size);auto list=reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());InitializeProcThreadAttributeList(list,1,0,&size);
    UpdateProcThreadAttribute(list,0,PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,console,sizeof(console),nullptr,nullptr);
    STARTUPINFOEXW si{};si.StartupInfo.cb=sizeof(si);si.StartupInfo.dwFlags=STARTF_USESTDHANDLES;si.lpAttributeList=list;std::wstring line=argv_quote(image)+L" "+args;PROCESS_INFORMATION pi{};
    if(!CreateProcessW(image.c_str(),line.data(),nullptr,nullptr,FALSE,EXTENDED_STARTUPINFO_PRESENT,nullptr,cwd.c_str(),&si.StartupInfo,&pi)){*exit_code=-2;return "conpty-spawn-failed:"+std::to_string(GetLastError());}
    std::string text;
    HANDLE reader=CreateThread(nullptr,0,[](void* value)->DWORD{auto* pair=static_cast<std::pair<HANDLE,std::string*>*>(value);char b[4096];DWORD n=0;while(ReadFile(pair->first,b,sizeof(b),&n,nullptr)&&n)pair->second->append(b,n);return 0;},new std::pair<HANDLE,std::string*>(out_r,&text),0,nullptr);
    WaitForSingleObject(pi.hProcess,60000);DWORD code=999;GetExitCodeProcess(pi.hProcess,&code);*exit_code=static_cast<int>(code);
    Sleep(500);ClosePseudoConsole(console);CloseHandle(out_w);WaitForSingleObject(reader,5000);
    CloseHandle(pi.hProcess);CloseHandle(pi.hThread);DeleteProcThreadAttributeList(list);return text;
}
int provider(const std::vector<std::string>& a) {
    const auto& ws=a[0];const auto& pub=a[2];const uint16_t unrelated=static_cast<uint16_t>(std::stoi(a[3]));const auto& probe=a[6];const auto& runtime=a[7];const auto& pwsh=a[8];
    const auto origin=getenv_utf8(L"ANTHROPIC_BASE_URL");const auto token=getenv_utf8(L"ANTHROPIC_AUTH_TOKEN");const auto proxy=origin_port(origin);
    std::vector<std::string> out;auto say=[&](const std::string& s){out.push_back(s);};
    std::string status;const int gateway=gateway_http(proxy,token,&status);
    say("A.gateway_connect="+std::to_string(gateway));say("A.gateway_status="+status);
    say("A.public_tcp="+std::to_string(connect_error(pub,443)));say("A.unrelated_loopback="+std::to_string(connect_error("127.0.0.1",unrelated)));
    say("A.dns="+std::to_string(resolve_error("www.microsoft.com")));
    write_text(ws+"\\from-a.txt","written-by-a");
    const auto bash=wide(getenv_utf8(L"CLAUDE_CODE_GIT_BASH_PATH"));const auto workspace=wide(ws);
    say("A.bash_path="+narrow(bash));say("A.comspec="+getenv_utf8(L"ComSpec"));say("A.path="+getenv_utf8(L"PATH"));
    std::string script=single(posix(probe))+" command";
    for(const auto& value:{a[0],a[1],pub,a[3],a[4],a[5],std::to_string(proxy),runtime,std::to_string(GetCurrentProcessId()),a[9]})script+=" "+single(value);
    script+="; export -p > "+single(posix(ws)+"/b-env.txt")+"; echo B.curl_http=$(curl.exe -sS -o /dev/null -w %{http_code}/%{size_download} --max-time 20 https://raw.githubusercontent.com/git/git/v2.40.0/COPYING 2>&1); echo B.nested_cmd=$(\"$COMSPEC\" //d //c \"echo ok\" 2>&1); echo B.curl_gateway=$(curl.exe -sS -o /dev/null -w %{http_code} --max-time 5 "+origin+" 2>&1); exit 37";
    auto cmdb=spawn_shim(bash,L"-c "+argv_quote(wide(script)),workspace);const auto bout=drain(cmdb.out);const int bcode=wait_exit(cmdb);
    say(bout);say("A.bash_exit="+std::to_string(bcode));
    const auto benv=read_text(ws+"\\b-env.txt");
    say(std::string("A.benv_read=")+(benv.empty()||benv=="<denied>"?"no":"yes"));
    say(std::string("A.benv_token_absent=")+(benv.find(token)==std::string::npos?"yes":"no"));
    say(std::string("A.benv_origin_absent=")+(benv.find(origin)==std::string::npos?"yes":"no"));
    say(std::string("A.benv_prefix_absent=")+(benv.find("ANTHROPIC_")==std::string::npos&&benv.find("GENT_")==std::string::npos&&benv.find("PROXY")==std::string::npos?"yes":"no"));
    say("A.reads_from_b="+read_text(ws+"\\from-b.txt"));
    say(std::string("A.overwrite_from_b=")+(write_text(ws+"\\from-b.txt","a-overwrote-b")?"ok":"denied"));
    say(std::string("A.delete_b_file=")+(DeleteFileW(wide(ws+"\\from-b-delete.txt").c_str())?"ok":"denied"));
    auto comspec=spawn_shim(wide(getenv_utf8(L"ComSpec")),L"/d /s /c \"exit 23\"",workspace);drain(comspec.out);say("A.cmd_exit="+std::to_string(wait_exit(comspec)));
    if(pwsh!="-"){auto ps=spawn_shim(wide(pwsh),L"-NoProfile -Command \"exit 29\"",workspace);const auto text=drain(ps.out);say("A.pwsh_exit="+std::to_string(wait_exit(ps)));if(!text.empty())say("A.pwsh_output="+text);
        auto refused=spawn_shim(wide(pwsh),L"-File x.ps1",workspace);const auto refused_out=drain(refused.out);say("A.pwsh_bad_args_exit="+std::to_string(wait_exit(refused)));say("A.pwsh_bad_args_output="+refused_out);}
    int conpty_exit=0;const auto conpty_out=conpty_run(bash,L"-c "+argv_quote(L"echo B.conpty=ok; read -t 2 ignored; exit 41"),workspace,&conpty_exit);
    int control_exit=0;const auto control_out=conpty_run(wide(probe),L"conout",workspace,&control_exit);
    say("A.conpty_control_exit="+std::to_string(control_exit));say(std::string("A.conpty_control_output=")+(control_out.find("control=ok")!=std::string::npos?"yes":"no:"+control_out));
    say("A.conpty_exit="+std::to_string(conpty_exit));say(std::string("A.conpty_output_relayed=")+(conpty_out.find("B.conpty=ok")!=std::string::npos?"yes":"no:"+conpty_out));
    auto longrun=spawn_shim(bash,L"-c "+argv_quote(wide(single(posix(probe))+" sleeper "+single(posix(ws)+"/sleeper.pid"))),workspace);
    say(std::string("A.sleeper_started=")+(wait_for_file(ws+"\\sleeper.pid",20000)?"yes":"no"));
    TerminateProcess(longrun.pi.hProcess,9);wait_exit(longrun);CloseHandle(longrun.out);write_text(ws+"\\shim-killed","x");
    auto bg=spawn_shim(bash,L"-c "+argv_quote(wide("("+single(posix(probe))+" sleeper "+single(posix(ws)+"/background.pid")+" >/dev/null 2>&1 </dev/null &) ; exit 0")),workspace);
    drain(bg.out);say("A.background_shell_exit="+std::to_string(wait_exit(bg)));
    say(std::string("A.background_started=")+(wait_for_file(ws+"\\background.pid",20000)?"yes":"no"));
    say(std::string("A.host_checked=")+(wait_for_file(ws+"\\host-checked",30000)?"yes":"no"));
    std::string report;for(const auto& line:out){report+=line;report+="\n";}
    write_text(ws+"\\provider-report.txt",report);return 0;
}
int command(const std::vector<std::string>& a) {
    const auto& ws=a[0];const auto& outside=a[1];const auto& pub=a[2];const uint16_t unrelated=static_cast<uint16_t>(std::stoi(a[3]));const auto& lan=a[4];const uint16_t lanport=static_cast<uint16_t>(std::stoi(a[5]));const uint16_t proxy=static_cast<uint16_t>(std::stoi(a[6]));const DWORD provider_pid=static_cast<DWORD>(std::stoul(a[8]));const uint16_t host_gateway=static_cast<uint16_t>(std::stoi(a[9]));
    emit("B.stdout_through_duplicated_handle=ok");
    emit("B.public_tcp="+std::to_string(connect_error(pub,443)));emit("B.dns="+std::to_string(resolve_error("www.microsoft.com")));
    emit("B.gateway_proxy_port="+std::to_string(connect_error("127.0.0.1",proxy)));emit("B.host_gateway_port="+std::to_string(connect_error("127.0.0.1",host_gateway)));emit("B.unrelated_loopback="+std::to_string(connect_error("127.0.0.1",unrelated)));
    emit("B.host_lan="+std::to_string(connect_error(lan,lanport)));
    emit(std::string("B.outside_read=")+(can_read(outside+"\\secret.txt")?"allowed":"denied"));emit(std::string("B.outside_write=")+(can_write(outside+"\\b.txt")?"allowed":"denied"));
    const auto claude=getenv_utf8(L"CLAUDE_CONFIG_DIR");emit(std::string("B.claude_config_read=")+(can_read(claude+"\\marker.txt")?"allowed":"denied"));emit(std::string("B.claude_config_write=")+(can_write(claude+"\\b.txt")?"allowed":"denied"));
    const auto codex=getenv_utf8(L"CODEX_HOME");emit(std::string("B.codex_config_read=")+(can_read(codex+"\\marker.txt")?"allowed":"denied"));emit(std::string("B.codex_config_write=")+(can_write(codex+"\\b.txt")?"allowed":"denied"));
    const auto home=getenv_utf8(L"USERPROFILE");emit(std::string("B.home_write=")+(can_write(home+"\\b.txt")?"allowed":"denied"));
    emit(std::string("B.claurst_config_read=")+(can_read(home+"\\.claurst\\marker.txt")?"allowed":"denied"));emit(std::string("B.claurst_config_write=")+(can_write(home+"\\.claurst\\b.txt")?"allowed":"denied"));
    emit(std::string("B.tmp_write=")+(can_write(getenv_utf8(L"TEMP")+"\\b.txt")?"allowed":"denied"));emit("B.temp_value="+getenv_utf8(L"TEMP"));emit("B.comspec="+getenv_utf8(L"ComSpec"));emit("B.path="+getenv_utf8(L"PATH"));
    wchar_t temp[MAX_PATH]{};GetTempPathW(MAX_PATH,temp);emit(std::string("B.os_temp_write=")+(can_write(narrow(temp)+"b.txt")?"allowed":"denied"));
    wchar_t sys[MAX_PATH]{};GetSystemDirectoryW(sys,MAX_PATH);emit(std::string("B.system32_read=")+(can_read(narrow(sys)+"\\kernel32.dll")?"allowed":"denied"));emit(std::string("B.system32_write=")+(can_write(narrow(sys)+"\\gent-b.txt")?"allowed":"denied"));
    HANDLE a_process=OpenProcess(PROCESS_VM_READ|PROCESS_QUERY_INFORMATION,FALSE,provider_pid);emit(std::string("B.open_provider_process=")+(a_process?"allowed":"denied"));if(a_process)CloseHandle(a_process);
    HANDLE pipe=CreateFileW(L"\\\\.\\pipe\\LOCAL\\gent-command-broker",GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr);emit(std::string("B.broker_pipe=")+(pipe==INVALID_HANDLE_VALUE?"denied":"allowed"));if(pipe!=INVALID_HANDLE_VALUE)CloseHandle(pipe);
    emit("B.reads_from_a="+read_text(ws+"\\from-a.txt"));emit(std::string("B.overwrite_from_a=")+(write_text(ws+"\\from-a.txt","b-overwrote-a")?"ok":"denied"));
    emit(std::string("B.create_from_b=")+(write_text(ws+"\\from-b.txt","written-by-b")?"ok":"denied"));write_text(ws+"\\from-b-delete.txt","x");
    return 0;
}
int sleeper(const std::string& pid_file){write_text(pid_file,std::to_string(GetCurrentProcessId()));Sleep(600000);return 0;}
}
int wmain(int argc,wchar_t** argv) {
    WSADATA data{};WSAStartup(MAKEWORD(2,2),&data);
    std::vector<std::string> args;for(int i=2;i<argc;++i)args.push_back(narrow(argv[i]));
    const std::wstring mode=argc>1?argv[1]:L"";
    if(mode==L"provider"&&args.size()==10)return provider(args);
    if(mode==L"command"&&args.size()==10)return command(args);
    if(mode==L"script"&&args.size()==2){auto child=spawn_shim(wide(getenv_utf8(L"CLAUDE_CODE_GIT_BASH_PATH")),L"-c "+argv_quote(wide(args[0])),wide(args[1]));const auto text=drain(child.out);const int code=wait_exit(child);emit(text);return code;}
    if(mode==L"open"){for(const auto& path:args){const auto file=CreateFileW(wide(path).c_str(),GENERIC_READ,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS,nullptr);emit(path+" read="+std::to_string(file==INVALID_HANDLE_VALUE?GetLastError():0));if(file!=INVALID_HANDLE_VALUE)CloseHandle(file);}return 0;}
    if(mode==L"script-real"&&args.size()==3){auto child=spawn_shim(wide(args[2]),L"-c "+argv_quote(wide(args[0])),wide(args[1]));const auto text=drain(child.out);const int code=wait_exit(child);emit(text);return code;}
    if(mode==L"sleeper"&&args.size()==1)return sleeper(args[0]);
    if(mode==L"conout"){emit("control=ok");Sleep(1500);return 7;}
    std::fputs("two-env-probe: bad mode\n",stderr);return 2;
}

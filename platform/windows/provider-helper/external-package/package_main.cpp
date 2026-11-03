#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <shellapi.h>
#include <sddl.h>
#include <appmodel.h>
#include <string>
#include <vector>
#include "external_channel_client.h"
#include "package_server.h"
#include "package_startup.h"
static std::string ascii(const std::wstring&s){std::string o;for(wchar_t c:s){if(c>127)return {};o.push_back(static_cast<char>(c));}return o;}
static bool write_receipt(const std::wstring&path,const std::string&data){HANDLE f=CreateFileW(path.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);if(f==INVALID_HANDLE_VALUE)return false;DWORD n=0;bool ok=WriteFile(f,data.data(),static_cast<DWORD>(data.size()),&n,nullptr)&&n==data.size();CloseHandle(f);return ok;}
static bool descriptor_text(const std::wstring&path,std::string&out){HANDLE f=CreateFileW(path.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL|FILE_FLAG_OPEN_REPARSE_POINT,nullptr);if(f==INVALID_HANDLE_VALUE)return false;FILE_ATTRIBUTE_TAG_INFO info{};LARGE_INTEGER size{};bool ok=GetFileType(f)==FILE_TYPE_DISK&&GetFileInformationByHandleEx(f,FileAttributeTagInfo,&info,sizeof(info))&&!(info.FileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY))&&GetFileSizeEx(f,&size)&&size.QuadPart>0&&size.QuadPart<=1024;if(ok){out.resize(static_cast<size_t>(size.QuadPart));DWORD n=0;ok=ReadFile(f,out.data(),static_cast<DWORD>(out.size()),&n,nullptr)&&n==out.size()&&std::all_of(out.begin(),out.end(),[](unsigned char c){return c>0&&c<128;});}CloseHandle(f);return ok;}
int WINAPI wWinMain(HINSTANCE,HINSTANCE,PWSTR,int){
 int argc=0;LPWSTR*argv=CommandLineToArgvW(GetCommandLineW(),&argc);if(!argv||argc!=7)return 10;
 const std::wstring receipt=argv[1],descriptor=argv[2];const std::string nonce=ascii(argv[3]);external_bridge::ClientBinding binding{wcstoul(argv[4],nullptr,10),_wcstoui64(argv[5],nullptr,10),wcstoul(argv[6],nullptr,10),nonce};LocalFree(argv);
 if(nonce.size()!=32||!binding.owner_pid||!binding.owner_birth)return 11;
 HANDLE owner=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,binding.owner_pid);if(!owner||external_bridge::process_birth(owner)!=binding.owner_birth||WaitForSingleObject(owner,0)!=WAIT_TIMEOUT)return 12;
 DWORD session=0;if(!ProcessIdToSessionId(GetCurrentProcessId(),&session)||session!=binding.session)return 13;
 HANDLE job=CreateJobObjectW(nullptr,nullptr);JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE|JOB_OBJECT_LIMIT_ACTIVE_PROCESS|JOB_OBJECT_LIMIT_PROCESS_MEMORY;limits.BasicLimitInformation.ActiveProcessLimit=1;limits.ProcessMemoryLimit=64*1024*1024;if(!job||!SetInformationJobObject(job,JobObjectExtendedLimitInformation,&limits,sizeof(limits))||!AssignProcessToJobObject(job,GetCurrentProcess()))return 14;
 HANDLE token=nullptr;if(!OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token))return 15;DWORD ac=0,n=sizeof(ac);if(!GetTokenInformation(token,TokenIsAppContainer,&ac,n,&n)||!ac)return 16;GetTokenInformation(token,TokenAppContainerSid,nullptr,0,&n);std::vector<BYTE>data(n);if(!GetTokenInformation(token,TokenAppContainerSid,data.data(),n,&n))return 17;LPWSTR sid=nullptr;if(!ConvertSidToStringSidW(reinterpret_cast<TOKEN_APPCONTAINER_INFORMATION*>(data.data())->TokenAppContainer,&sid))return 18;UINT32 count=0;if(GetCurrentPackageFamilyName(&count,nullptr)!=ERROR_INSUFFICIENT_BUFFER)return 19;std::vector<wchar_t>family(count);if(GetCurrentPackageFamilyName(&count,family.data()))return 20;
 const std::string identity="\"pid\":"+std::to_string(GetCurrentProcessId())+",\"birth\":"+std::to_string(external_bridge::process_birth(GetCurrentProcess()))+",\"session\":"+std::to_string(session)+",\"appContainer\":true,\"sid\":\""+ascii(sid)+"\",\"pfn\":\""+ascii(family.data())+"\",\"nonce\":\""+nonce+"\"";LocalFree(sid);CloseHandle(token);
 const auto admission="GENT-PACKAGE-ADMISSION/1 "+std::to_string(GetCurrentProcessId())+" "+std::to_string(external_bridge::process_birth(GetCurrentProcess()))+" "+std::to_string(binding.owner_pid)+" "+std::to_string(binding.owner_birth)+" "+std::to_string(session)+" "+nonce+"\n";
 HANDLE admitted=CreateFileW((receipt+L".admission").c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL|FILE_FLAG_WRITE_THROUGH,nullptr);if(admitted==INVALID_HANDLE_VALUE)return 21;DWORD admission_written=0;bool published=WriteFile(admitted,admission.data(),static_cast<DWORD>(admission.size()),&admission_written,nullptr)&&admission_written==admission.size()&&FlushFileBuffers(admitted);CloseHandle(admitted);if(!published)return 21;
 if(!write_receipt(receipt,"{\"phase\":\"awaitingDescriptor\","+identity+"}"))return 21;
 std::vector<HANDLE> channels;const ULONGLONG deadline=GetTickCount64()+15000;
 while(GetTickCount64()<deadline&&WaitForSingleObject(owner,0)==WAIT_TIMEOUT){std::string text;if(descriptor_text(descriptor,text)){channels=external_bridge::admit_package_channels(text,binding,owner);if(channels.size()!=9){write_receipt(receipt+L".done.json","{\"phase\":\"rejectedDescriptor\","+identity+"}");return 22;}break;}Sleep(10);}
 if(channels.size()!=9){write_receipt(receipt+L".done.json","{\"phase\":\"descriptorTimeout\","+identity+"}");return 23;}
 const HANDLE control=channels.front();channels.erase(channels.begin());
 DWORD quota=0;std::string bearer;bool ready=GetNamedPipeInfo(control,nullptr,&quota,nullptr,nullptr)&&quota>=83&&external_bridge::consume_challenge(control,owner,binding)&&external_bridge::read_run_bearer(control,owner,bearer);
 if(ready)ready=external_bridge::serve_package_channels(channels,owner,bearer,[&](uint16_t port){
  const auto status="GENT-BRIDGE-READY/1 "+nonce+" "+std::to_string(port)+"\n";
  DWORD written=0;return WriteFile(control,status.data(),static_cast<DWORD>(status.size()),&written,nullptr)&&written==status.size();
 });
 if(!bearer.empty())SecureZeroMemory(bearer.data(),bearer.size());bearer.clear();CloseHandle(control);for(auto channel:channels)CloseHandle(channel);
 write_receipt(receipt+L".done.json","{\"phase\":\""+std::string(ready?"retired":"failed")+"\","+identity+"}");CloseHandle(owner);
 // The OS closes this self-owned kill-on-close Job at process exit.
 return ready?0:24;
}

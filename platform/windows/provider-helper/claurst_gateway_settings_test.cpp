#include <windows.h>
#include <string>
#include <stdexcept>
#include <fstream>
struct Handle { HANDLE h; explicit Handle(HANDLE v):h(v){} ~Handle(){if(h!=INVALID_HANDLE_VALUE)CloseHandle(h);} };
static void check(bool ok,const char* label){if(!ok)throw std::runtime_error(label);}
#include "claurst_gateway_settings.h"
int wmain(int argc,wchar_t** argv) {
    check(argc==2,"owned-fixture-argument");
    const std::wstring root=argv[1];check(CreateDirectoryW(root.c_str(),nullptr),"fresh-owned-fixture");
    const std::string marker="http://127.0.0.1:0/__gent_claurst_proxy_v1__/v1";
    check(claurst_gateway::project("\""+marker+"\"",55490)=="\"http://127.0.0.1:55490/v1\"","exact-port");
    for(const auto& bad : {std::string("{}"),marker,"\""+marker+"\" \""+marker+"\"",std::string("__gent_claurst_proxy_v1__ \"")+marker+"\""}) {
        bool rejected=false;try{(void)claurst_gateway::project(bad,55490);}catch(const std::runtime_error&){rejected=true;}
        check(rejected,"invalid-template-admitted");
    }
    bool rejected=false;try{(void)claurst_gateway::project("\""+marker+"\"",0);}catch(const std::runtime_error&){rejected=true;}
    check(rejected,"zero-port-admitted");
    const auto path=root+L"\\settings \u00f1.json";
    const std::string original="{\"api_base\":\""+marker+"\"}";
    {std::ofstream file(path);file<<original;check(bool(file),"write-fixture");}
    project_claurst_gateway_settings(path,55500);
    {std::ifstream file(path);std::string got;std::getline(file,got);check(got=="{\"api_base\":\"http://127.0.0.1:55500/v1\"}","file-projection");}
    rejected=false;try{project_claurst_gateway_settings(path,55501);}catch(const std::runtime_error&){rejected=true;}
    check(rejected,"repeat-admitted");
    {std::ofstream file(path);file<<original;check(bool(file),"write-hardlink-fixture");}
    const auto alias=path+L".alias";check(CreateHardLinkW(alias.c_str(),path.c_str(),nullptr),"hardlink-fixture");
    rejected=false;try{project_claurst_gateway_settings(path,55502);}catch(const std::runtime_error&){rejected=true;}
    check(rejected,"hardlink-admitted");
    {std::ifstream file(path);std::string got;std::getline(file,got);check(got==original,"hardlink-mutated");}
    check(DeleteFileW(alias.c_str()),"alias-cleanup");check(DeleteFileW(path.c_str()),"file-cleanup");
    check(RemoveDirectoryW(root.c_str()),"root-cleanup");
}

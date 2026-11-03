#include "claurst_mcp_environment.h"
#include <iostream>
using gent_claurst_mcp_environment::append;
static void require(bool value){if(!value)throw std::runtime_error("test");}
int main(){
 try {
    const std::wstring run(64,L'a'),broker(64,L'b');std::vector<std::wstring> env{L"TERM=dumb",L"gent_mcp_bridge_token=stale",L"Gent_Mcp_Bridge_Origin=http://wrong"};
    append(env,L"http://127.0.0.1:55490",run);require(env.size()==3);require(env[0]==L"TERM=dumb");require(env[1]==L"GENT_MCP_BRIDGE_ORIGIN=http://127.0.0.1:55490");require(env[2]==L"GENT_MCP_BRIDGE_TOKEN="+run);require(env[2]!=L"GENT_MCP_BRIDGE_TOKEN="+broker);
    unsigned cases=1;
    for(const auto* origin:{L"http://192.168.1.1:1",L"http://localhost:1",L"http://127.0.0.1:0",L"http://127.0.0.1:65536",L"http://127.0.0.1:1/path",L"http://127.0.0.1:-1"}){const auto before=env;bool failed=false;try{append(env,origin,run);}catch(const std::runtime_error&){failed=true;}require(failed&&env==before);++cases;}
    for(const auto& token:{std::wstring(63,L'a'),std::wstring(64,L'z'),std::wstring(64,L'\n')}){const auto before=env;bool failed=false;try{append(env,L"http://127.0.0.1:1",token);}catch(const std::runtime_error&){failed=true;}require(failed&&env==before);++cases;}
    std::cout<<cases<<" native environment boundary cases PASS\n";return 0;
 }catch(...){return 1;}
}

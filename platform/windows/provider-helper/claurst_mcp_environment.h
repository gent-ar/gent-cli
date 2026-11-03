#pragma once
#include <algorithm>
#include <cwchar>
#include <stdexcept>
#include <string>
#include <vector>
namespace gent_claurst_mcp_environment {
inline bool key(const std::wstring& entry,const wchar_t* expected) {
    const auto equal=entry.find(L'=');return equal!=std::wstring::npos&&equal==std::wcslen(expected)&&_wcsnicmp(entry.c_str(),expected,equal)==0;
}
inline void append(std::vector<std::wstring>& env,const std::wstring& origin,const std::wstring& run_bearer) {
    const std::wstring prefix=L"http://127.0.0.1:";
    if(origin.rfind(prefix,0)!=0||origin.size()<=prefix.size()||origin.size()>prefix.size()+5)
        throw std::runtime_error("claurst-mcp-origin");
    unsigned port=0;for(auto value:origin.substr(prefix.size())) {if(value<L'0'||value>L'9')throw std::runtime_error("claurst-mcp-origin");port=port*10+unsigned(value-L'0');}
    if(!port||port>65535)throw std::runtime_error("claurst-mcp-origin");
    if(run_bearer.size()!=64||!std::all_of(run_bearer.begin(),run_bearer.end(),[](wchar_t value){return (value>=L'0'&&value<=L'9')||(value>=L'a'&&value<=L'f')||(value>=L'A'&&value<=L'F');}))
        throw std::runtime_error("claurst-mcp-bearer");
    env.erase(std::remove_if(env.begin(),env.end(),[](const auto& value){return key(value,L"GENT_MCP_BRIDGE_ORIGIN")||key(value,L"GENT_MCP_BRIDGE_TOKEN");}),env.end());
    env.push_back(L"GENT_MCP_BRIDGE_ORIGIN="+origin);
    env.push_back(L"GENT_MCP_BRIDGE_TOKEN="+run_bearer);
}
}

#pragma once
#include <algorithm>
#include <cwchar>
#include <string>
#include <vector>
namespace gent_command {
struct EnvironmentPolicy {
    std::wstring shim_directory,windows_directory,bearer,gateway_origin,temp_root,appdata,localappdata,git_ceiling,bash_image,powershell_image;
    std::vector<std::wstring> real_shell_directories,tool_directories;
};
inline bool starts_with_ignore_case(const std::wstring& value,const wchar_t* prefix) {
    const auto size=std::wcslen(prefix);return value.size()>=size&&_wcsnicmp(value.c_str(),prefix,size)==0;
}
inline bool same_directory(std::wstring left,std::wstring right) {
    while(left.size()>3&&(left.back()==L'\\'||left.back()==L'/'))left.pop_back();
    while(right.size()>3&&(right.back()==L'\\'||right.back()==L'/'))right.pop_back();
    return _wcsicmp(left.c_str(),right.c_str())==0;
}
inline bool provider_only_variable(const std::wstring& entry) {
    for(const auto* prefix:{L"ANTHROPIC_",L"OPENAI_",L"GENT_",L"CUSTOM_OPENAI_",L"CLAURST_",L"LLAMA_CPP_"})if(starts_with_ignore_case(entry,prefix))return true;
    for(const auto* name:{L"HTTP_PROXY=",L"HTTPS_PROXY=",L"ALL_PROXY=",L"NO_PROXY=",L"FTP_PROXY="})if(starts_with_ignore_case(entry,name))return true;
    const auto name=entry.substr(0,entry.find(L'='));
    for(const auto* suffix:{L"_TOKEN",L"_API_KEY",L"_SECRET"}) {
        const auto size=std::wcslen(suffix);
        if(name.size()>=size&&_wcsicmp(name.c_str()+name.size()-size,suffix)==0)return true;
    }
    return false;
}
inline std::wstring rewritten_path(const std::wstring& value,const EnvironmentPolicy& policy) {
    std::vector<std::wstring> entries;
    size_t start=0;
    while(start<=value.size()) {
        auto end=value.find(L';',start);if(end==std::wstring::npos)end=value.size();
        const auto part=value.substr(start,end-start);start=end+1;
        if(part.empty())continue;
        if(same_directory(part,policy.shim_directory))entries.insert(entries.end(),policy.real_shell_directories.begin(),policy.real_shell_directories.end());
        else entries.push_back(part);
    }
    for(const auto& required:{policy.windows_directory+L"\\System32",policy.windows_directory}) {
        if(std::none_of(entries.begin(),entries.end(),[&](const auto& entry){return same_directory(entry,required);}))entries.push_back(required);
    }
    for(const auto& tool:policy.tool_directories) {
        if(std::none_of(entries.begin(),entries.end(),[&](const auto& entry){return same_directory(entry,tool);}))entries.push_back(tool);
    }
    std::wstring result;for(const auto& entry:entries){if(!result.empty())result+=L';';result+=entry;}
    return result;
}
inline std::wstring without_shim_image(const std::wstring& entry,const EnvironmentPolicy& policy) {
    const auto equal=entry.find(L'=');
    if(equal==std::wstring::npos||equal==0)return entry;
    const auto value=entry.substr(equal+1);
    const auto key=entry.substr(0,equal+1);
    if(_wcsicmp(value.c_str(),(policy.shim_directory+L"\\bash.exe").c_str())==0)return key+policy.bash_image;
    if(_wcsicmp(value.c_str(),(policy.shim_directory+L"\\pwsh.exe").c_str())==0)return key+policy.powershell_image;
    if(_wcsicmp(value.c_str(),(policy.shim_directory+L"\\cmd.exe").c_str())==0)return key+policy.windows_directory+L"\\System32\\cmd.exe";
    return entry;
}
inline void route_git_http_through_resolver_helper(std::vector<std::wstring>& entries) {
    unsigned long count=0;
    for(auto entry=entries.begin();entry!=entries.end();++entry)if(starts_with_ignore_case(*entry,L"GIT_CONFIG_COUNT=")) {
        count=std::wcstoul(entry->c_str()+17,nullptr,10);entries.erase(entry);break;
    }
    for(const auto* scheme:{L"https://",L"http://"}) {
        const auto index=std::to_wstring(count++);
        entries.push_back(L"GIT_CONFIG_KEY_"+index+L"=url.gent::"+scheme+L".insteadOf");
        entries.push_back(L"GIT_CONFIG_VALUE_"+index+L"="+scheme);
    }
    entries.push_back(L"GIT_CONFIG_COUNT="+std::to_wstring(count));
}
inline void resolve_node_modules_without_drive_root_realpath(std::vector<std::wstring>& entries) {
    std::wstring options=L"--preserve-symlinks --preserve-symlinks-main";
    for(auto entry=entries.begin();entry!=entries.end();++entry)if(starts_with_ignore_case(*entry,L"NODE_OPTIONS=")) {
        const auto existing=entry->substr(13);if(!existing.empty())options=existing+L" "+options;
        entries.erase(entry);break;
    }
    entries.push_back(L"NODE_OPTIONS="+options);
}
inline std::vector<wchar_t> command_environment(const std::wstring& block,const EnvironmentPolicy& policy) {
    std::vector<std::wstring> entries;std::wstring path;
    for(size_t start=0;start<block.size();) {
        auto end=block.find(L'\0',start);if(end==std::wstring::npos)end=block.size();
        const auto entry=block.substr(start,end-start);start=end+1;
        if(entry.empty())continue;
        if(provider_only_variable(entry))continue;
        if(!policy.bearer.empty()&&entry.find(policy.bearer)!=std::wstring::npos)continue;
        if(!policy.gateway_origin.empty()&&entry.find(policy.gateway_origin)!=std::wstring::npos)continue;
        if(starts_with_ignore_case(entry,L"PATH=")){path=entry.substr(5);continue;}
        bool overridden=false;
        for(const auto* name:{L"LOCALAPPDATA=",L"APPDATA=",L"TEMP=",L"TMP=",L"TMPDIR=",L"COMSPEC=",L"PATHEXT=",L"GIT_CEILING_DIRECTORIES="})overridden=overridden||starts_with_ignore_case(entry,name);
        if(!overridden)entries.push_back(without_shim_image(entry,policy));
    }
    route_git_http_through_resolver_helper(entries);
    resolve_node_modules_without_drive_root_realpath(entries);
    entries.push_back(L"PATH="+rewritten_path(path,policy));
    entries.push_back(L"ComSpec="+policy.windows_directory+L"\\System32\\cmd.exe");entries.push_back(L"PATHEXT=.COM;.EXE;.BAT;.CMD");entries.push_back(L"GIT_CEILING_DIRECTORIES="+policy.git_ceiling);entries.push_back(L"LOCALAPPDATA="+policy.localappdata);entries.push_back(L"APPDATA="+policy.appdata);
    for(const auto* name:{L"TEMP=",L"TMP=",L"TMPDIR="})entries.push_back(name+policy.temp_root);
    std::sort(entries.begin(),entries.end(),[](const auto& a,const auto& b){return _wcsicmp(a.c_str(),b.c_str())<0;});
    std::vector<wchar_t> result;
    for(const auto& entry:entries){result.insert(result.end(),entry.begin(),entry.end());result.push_back(0);}
    result.push_back(0);return result;
}
inline bool command_directory_allowed(const std::wstring& cwd,const std::vector<std::wstring>& roots) {
    if(cwd.size()<3||cwd[1]!=L':'||cwd[2]!=L'\\')return false;
    std::wstring normalized=cwd;for(auto& c:normalized)if(c==L'/')c=L'\\';
    while(normalized.size()>3&&normalized.back()==L'\\')normalized.pop_back();
    for(size_t start=3;start<normalized.size();) {
        auto end=normalized.find(L'\\',start);if(end==std::wstring::npos)end=normalized.size();
        const auto part=normalized.substr(start,end-start);start=end+1;
        if(part.empty()||part==L"."||part==L"..")return false;
    }
    return std::any_of(roots.begin(),roots.end(),[&](const auto& root){
        return normalized.size()>=root.size()&&_wcsnicmp(normalized.c_str(),root.c_str(),root.size())==0&&(normalized.size()==root.size()||normalized[root.size()]==L'\\');});
}
}

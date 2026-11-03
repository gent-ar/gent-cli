#pragma once
#include <windows.h>
#include <string>
#include <stdexcept>
#include <cstring>
#include "win32_paths.h"
namespace gent_environment_paths {
inline void require(bool value,const char* label){if(!value)throw std::runtime_error(label);}
struct Handle {
    HANDLE h=INVALID_HANDLE_VALUE;
    explicit Handle(HANDLE value):h(value){}
    ~Handle(){if(h&&h!=INVALID_HANDLE_VALUE)CloseHandle(h);}
    Handle(const Handle&)=delete;Handle& operator=(const Handle&)=delete;
};
inline size_t root_length(const std::wstring& value) {
    if(value.size()>=3&&((value[0]>=L'A'&&value[0]<=L'Z')||(value[0]>=L'a'&&value[0]<=L'z'))&&value[1]==L':'&&value[2]==L'\\')return 3;
    require(value.rfind(L"\\\\",0)==0&&value.rfind(L"\\\\?\\",0)!=0&&value.rfind(L"\\\\.\\",0)!=0,"environment-path-kind");
    const auto server=value.find(L'\\',2);require(server!=std::wstring::npos&&server>2,"environment-path-unc-server");
    const auto share=value.find(L'\\',server+1);require((share==std::wstring::npos?value.size():share)>server+1,"environment-path-unc-share");
    return share==std::wstring::npos?value.size():share;
}
inline void validate(const std::wstring& value) {
    const auto root=root_length(value);
    require(value.find_first_of(L"*?")==std::wstring::npos&&value.find(L'\0')==std::wstring::npos,"environment-path-wildcard");
    size_t start=root; if(start<value.size()&&value[start]==L'\\')++start;
    while(start<value.size()) {
        auto end=value.find(L'\\',start);if(end==std::wstring::npos)end=value.size();
        const auto part=value.substr(start,end-start);
        require(!part.empty()&&part!=L"."&&part!=L".."&&part.back()!=L'.'&&part.back()!=L' '&&part.find(L':')==std::wstring::npos,"environment-path-component");
        auto base=part.substr(0,part.find(L'.'));for(auto& c:base)if(c>=L'a'&&c<=L'z')c-=L'a'-L'A';
        const bool reserved=base==L"CON"||base==L"PRN"||base==L"AUX"||base==L"NUL"||
            (base.size()==4&&(base.rfind(L"COM",0)==0||base.rfind(L"LPT",0)==0)&&base[3]>=L'1'&&base[3]<=L'9');
        require(!reserved,"environment-path-device");start=end+1;
    }
}
inline std::wstring project(const std::wstring& physical) {
    std::wstring out;
    if(physical.rfind(L"\\\\?\\UNC\\",0)==0)out=L"\\\\"+physical.substr(8);
    else if(physical.rfind(L"\\\\?\\",0)==0)out=physical.substr(4);
    else out=physical;
    validate(out);return out;
}
inline void no_reparse(const std::wstring& candidate) {
    const auto root=root_length(candidate);
    for(size_t i=root;i<=candidate.size();++i) {
        if(i!=root&&i!=candidate.size()&&candidate[i]!=L'\\')continue;
        const auto prefix=gent_windows_paths::io_path(candidate.substr(0,i));
        const auto attributes=GetFileAttributesW(prefix.c_str());
        require(attributes!=INVALID_FILE_ATTRIBUTES&&!(attributes&FILE_ATTRIBUTE_REPARSE_POINT),"environment-path-reparse");
    }
}
inline FILE_ID_INFO identity(HANDLE handle) {
    BY_HANDLE_FILE_INFORMATION attributes{};require(GetFileInformationByHandle(handle,&attributes)!=0&&!(attributes.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT),"environment-path-authority");
    FILE_ID_INFO info{};require(GetFileInformationByHandleEx(handle,FileIdInfo,&info,sizeof(info))!=0,"environment-path-identity");return info;
}
inline std::wstring verified_candidate(HANDLE authority,const std::wstring& candidate) {
    validate(candidate);no_reparse(candidate);
    const auto io=gent_windows_paths::io_path(candidate);
    Handle opened(CreateFileW(io.c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
    require(opened.h!=INVALID_HANDLE_VALUE,"environment-path-open");const auto a=identity(authority),b=identity(opened.h);
    require(a.VolumeSerialNumber==b.VolumeSerialNumber&&std::memcmp(a.FileId.Identifier,b.FileId.Identifier,sizeof(a.FileId.Identifier))==0,"environment-path-target-changed");return candidate;
}
inline std::wstring ordinary(HANDLE authority) {
    const DWORD flags=FILE_NAME_NORMALIZED|VOLUME_NAME_DOS;const auto needed=GetFinalPathNameByHandleW(authority,nullptr,0,flags);
    require(needed&&needed<32768,"environment-path-final-size");std::wstring physical(needed,L'\0');
    const auto written=GetFinalPathNameByHandleW(authority,physical.data(),needed,flags);require(written&&written<needed,"environment-path-final");physical.resize(written);
    return verified_candidate(authority,project(physical));
}
}

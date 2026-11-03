#pragma once
#include <windows.h>
#include <string>
#include <stdexcept>
inline std::wstring scope_final_path(HANDLE handle) {
    const DWORD flags=FILE_NAME_NORMALIZED|VOLUME_NAME_GUID;
    const auto length=GetFinalPathNameByHandleW(handle,nullptr,0,flags);
    if(!length||length>32768)throw std::runtime_error("control-scope-final-path");
    std::wstring path(length,L'\0');const auto written=GetFinalPathNameByHandleW(handle,path.data(),length,flags);
    if(!written||written>=length)throw std::runtime_error("control-scope-final-path");
    path.resize(written);while(path.size()>1&&path.back()==L'\\')path.pop_back();return path;
}
inline bool scope_path_contains(const std::wstring& parent,const std::wstring& child) {
    return child.size()>=parent.size()&&_wcsnicmp(parent.c_str(),child.c_str(),parent.size())==0
        &&(child.size()==parent.size()||child[parent.size()]==L'\\');
}
inline bool control_scope_disjoint(const std::wstring& control,const std::wstring& grant) {
    return !scope_path_contains(control,grant)&&!scope_path_contains(grant,control);
}

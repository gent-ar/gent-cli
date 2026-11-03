#pragma once
#include <windows.h>
#include <array>
#include <cstdint>
#include <string>
#include <stdexcept>
#include <vector>
#include <algorithm>
inline void read_bootstrap_exact(HANDLE input, void* data, DWORD size) {
    auto* bytes=static_cast<unsigned char*>(data);DWORD done=0;
    while(done<size){DWORD got=0;if(!ReadFile(input,bytes+done,size-done,&got,nullptr)||!got)
        throw std::runtime_error("readonly-bootstrap-read");done+=got;}
}
inline uint32_t read_bootstrap_u32(HANDLE input) {
    std::array<unsigned char,4> b{};read_bootstrap_exact(input,b.data(),4);
    return uint32_t(b[0])|(uint32_t(b[1])<<8)|(uint32_t(b[2])<<16)|(uint32_t(b[3])<<24);
}
inline void read_readonly_bootstrap(HANDLE input,std::vector<std::wstring>& paths) {
    std::array<char,8> magic{};read_bootstrap_exact(input,magic.data(),8);
    if(magic!=std::array<char,8>{'G','E','N','T','R','O','0','1'})
        throw std::runtime_error("readonly-bootstrap-magic");
    const auto count=read_bootstrap_u32(input);
    if(!count||count>1024||paths.size()>1024-count)
        throw std::runtime_error("readonly-bootstrap-count");
    size_t total=12;
    for(uint32_t i=0;i<count;++i) {
        const auto size=read_bootstrap_u32(input);total+=4;
        if(!size||size>32768||total>512*1024||size>512*1024-total)
            throw std::runtime_error("readonly-bootstrap-size");
        total+=size;std::string utf8(size,'\0');read_bootstrap_exact(input,utf8.data(),size);
        if(utf8.find('\0')!=std::string::npos)throw std::runtime_error("readonly-bootstrap-nul");
        const int chars=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,utf8.data(),static_cast<int>(size),nullptr,0);
        if(!chars)throw std::runtime_error("readonly-bootstrap-utf8");
        std::wstring path(chars,L'\0');
        if(MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,utf8.data(),static_cast<int>(size),path.data(),chars)!=chars)
            throw std::runtime_error("readonly-bootstrap-utf8");
        paths.push_back(std::move(path));
    }
}

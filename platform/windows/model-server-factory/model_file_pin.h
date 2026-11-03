#pragma once
struct ModelFilePin {
    HANDLE handle=INVALID_HANDLE_VALUE;
    ~ModelFilePin(){if(handle!=INVALID_HANDLE_VALUE)CloseHandle(handle);}
    void verify(const wchar_t* path,const std::string& expected,uint64_t maximum){
        need(expected.size()==64&&expected.find_first_not_of("0123456789abcdef")==std::string::npos,"model-sha-format");
        handle=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);need(handle!=INVALID_HANDLE_VALUE,"model-pin-open");
        BY_HANDLE_FILE_INFORMATION info{};LARGE_INTEGER size{};need(GetFileInformationByHandle(handle,&info)&&GetFileSizeEx(handle,&size)&&!(info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT))&&size.QuadPart>0&&static_cast<uint64_t>(size.QuadPart)<=maximum,"model-pin-file");
        BCRYPT_ALG_HANDLE algorithm=nullptr;need(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)==0,"model-sha-provider");
        BCRYPT_HASH_HANDLE hash=nullptr;auto status=BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0);unsigned char bytes[65536],digest[32]{};DWORD count=0;bool read_ok=true;
        while(status==0){if(!ReadFile(handle,bytes,sizeof(bytes),&count,nullptr)){read_ok=false;break;}if(!count)break;status=BCryptHashData(hash,bytes,count,0);}
        if(status==0&&read_ok)status=BCryptFinishHash(hash,digest,32,0);if(hash)BCryptDestroyHash(hash);BCryptCloseAlgorithmProvider(algorithm,0);need(status==0&&read_ok,"model-sha-read");
        std::string actual;constexpr char hex[]="0123456789abcdef";for(auto byte:digest){actual+=hex[byte>>4];actual+=hex[byte&15];}need(actual==expected,"model-sha-mismatch");
    }
};
static std::wstring model_wide_path(const std::string& value){
    need(!value.empty()&&value.size()<=32767&&value.find('\0')==std::string::npos,"model-path-size");const auto n=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,value.data(),static_cast<int>(value.size()),nullptr,0);need(n>0,"model-path-utf8");std::wstring out(n,0);need(MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,value.data(),static_cast<int>(value.size()),out.data(),n)==n,"model-path-utf8");
    const auto offset=out.rfind(L"\\\\?\\",0)==0?4u:0u;need(out.size()>offset+3&&((out[offset]>=L'A'&&out[offset]<=L'Z')||(out[offset]>=L'a'&&out[offset]<=L'z'))&&out[offset+1]==L':'&&out[offset+2]==L'\\',"model-path-absolute");return out;
}

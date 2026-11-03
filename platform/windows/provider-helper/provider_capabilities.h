#pragma once
#include <windows.h>
#include <initializer_list>
#include <stdexcept>
#include <vector>
inline void validate_exact_capabilities(HANDLE token,std::initializer_list<const wchar_t*> names) {
    DWORD size=0; GetTokenInformation(token,TokenCapabilities,nullptr,0,&size);
    if(!size||size>65536)throw std::runtime_error("diagnostic-capability-size");
    std::vector<unsigned char> bytes(size);
    if(!GetTokenInformation(token,TokenCapabilities,bytes.data(),size,&size))throw std::runtime_error("diagnostic-capability-read");
    const auto groups=reinterpret_cast<const TOKEN_GROUPS*>(bytes.data());
    if(groups->GroupCount!=names.size())throw std::runtime_error("diagnostic-capability-count");
    for(const auto* name:names) {
        PSID* capability=nullptr;PSID* group=nullptr;DWORD capability_count=0,group_count=0;
        if(!DeriveCapabilitySidsFromName(name,&group,&group_count,&capability,&capability_count))throw std::runtime_error("diagnostic-capability-derive");
        DWORD matched=0,attributes=0;
        if(capability_count==1)for(DWORD n=0;n<groups->GroupCount;++n)if(EqualSid(groups->Groups[n].Sid,capability[0])){++matched;attributes=groups->Groups[n].Attributes;}
        for(DWORD n=0;n<capability_count;++n)LocalFree(capability[n]);LocalFree(capability);
        for(DWORD n=0;n<group_count;++n)LocalFree(group[n]);LocalFree(group);
        if(matched!=1||(attributes&SE_GROUP_USE_FOR_DENY_ONLY))throw std::runtime_error("diagnostic-capability-mismatch");
    }
}
inline void validate_provider_capabilities(HANDLE token,bool gateway) {
    if(gateway)validate_exact_capabilities(token,{L"AgenticAppContainer",L"privateNetworkClientServer"});
    else validate_exact_capabilities(token,{L"AgenticAppContainer"});
}
inline void validate_command_capabilities(HANDLE token) {
    validate_exact_capabilities(token,{L"AgenticAppContainer",L"internetClient"});
}
inline void validate_low_integrity(HANDLE token) {
    DWORD length=0;GetTokenInformation(token,TokenIntegrityLevel,nullptr,0,&length);
    if(!length)throw std::runtime_error("token-integrity");
    std::vector<unsigned char> bytes(length);
    if(!GetTokenInformation(token,TokenIntegrityLevel,bytes.data(),length,&length))throw std::runtime_error("token-integrity");
    const auto sid=reinterpret_cast<TOKEN_MANDATORY_LABEL*>(bytes.data())->Label.Sid;
    if(*GetSidSubAuthority(sid,static_cast<DWORD>(*GetSidSubAuthorityCount(sid)-1))>SECURITY_MANDATORY_LOW_RID)throw std::runtime_error("token-integrity-high");
}
inline std::vector<unsigned char> token_appcontainer_sid(HANDLE token) {
    DWORD is_container=0,size=0;
    if(!GetTokenInformation(token,TokenIsAppContainer,&is_container,sizeof(is_container),&size)||!is_container)throw std::runtime_error("token-not-appcontainer");
    size=0;GetTokenInformation(token,TokenAppContainerSid,nullptr,0,&size);
    if(!size||size>65536)throw std::runtime_error("token-appcontainer-size");
    std::vector<unsigned char> bytes(size);
    if(!GetTokenInformation(token,TokenAppContainerSid,bytes.data(),size,&size))throw std::runtime_error("token-appcontainer-read");
    const auto sid=reinterpret_cast<TOKEN_APPCONTAINER_INFORMATION*>(bytes.data())->TokenAppContainer;
    if(!IsValidSid(sid))throw std::runtime_error("token-appcontainer-invalid");
    std::vector<unsigned char> result(GetLengthSid(sid));
    if(!CopySid(static_cast<DWORD>(result.size()),result.data(),sid))throw std::runtime_error("token-appcontainer-copy");
    return result;
}

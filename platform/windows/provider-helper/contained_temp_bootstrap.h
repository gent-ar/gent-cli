#pragma once
static int prepare_temp_inside(const std::wstring& base) {
    HANDLE raw = nullptr; check(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw) != 0, "bootstrap-token"); Handle token(raw);
    DWORD app = 0, size = 0; check(GetTokenInformation(token.h, TokenIsAppContainer, &app, sizeof(app), &size) && app, "bootstrap-not-contained");
    wchar_t local[32768]{}, temp[32768]{}; DWORD l=GetEnvironmentVariableW(L"LOCALAPPDATA",local,32768), t=GetTempPathW(32768,temp);
    check(l && l<32768 && t && t<32768, "bootstrap-paths");
    std::wstring localpath=local, path=temp;
    while(!localpath.empty()&&localpath.back()==L'\\')localpath.pop_back();while(!path.empty()&&path.back()==L'\\')path.pop_back();
    check(_wcsicmp((localpath+L"\\Temp").c_str(),path.c_str())==0, "bootstrap-temp-layout");
    std::wstring prefix=base+L"\\Packages\\sandbox.";
    check(localpath.size()>prefix.size() && _wcsnicmp(localpath.c_str(),prefix.c_str(),prefix.size())==0,"bootstrap-state-prefix");
    auto suffix=localpath.substr(prefix.size()); check(suffix.size()==41 && suffix.substr(38)==L"\\AC", "bootstrap-container-layout");
    auto name=suffix.substr(0,38);check(name.front()==L'{'&&name.back()==L'}',"bootstrap-container-id");
    for(size_t i=1;i+1<name.size();++i) {wchar_t c=name[i];check((c>=L'0'&&c<=L'9')||(c>=L'a'&&c<=L'f')||(c>=L'A'&&c<=L'F')||c==L'-',"bootstrap-container-id-char");}
    std::vector<Handle> locked;
    auto hold_directory=[&](const std::wstring& dir, bool create) {
        auto io=gent_windows_paths::io_path(dir);
        if(create) check(CreateDirectoryW(io.c_str(),nullptr) || GetLastError()==ERROR_ALREADY_EXISTS,"bootstrap-directory-create");
        Handle handle(CreateFileW(io.c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
        check(handle.h!=INVALID_HANDLE_VALUE,"bootstrap-directory-open");FILE_ATTRIBUTE_TAG_INFO info{};
        check(GetFileInformationByHandleEx(handle.h,FileAttributeTagInfo,&info,sizeof(info)) && (info.FileAttributes&FILE_ATTRIBUTE_DIRECTORY) && !(info.FileAttributes&FILE_ATTRIBUTE_REPARSE_POINT),"bootstrap-directory-invalid");
        locked.push_back(std::move(handle));
    };
    hold_directory(base,false);
    for(const auto& dir : {base+L"\\Packages",base+L"\\Packages\\sandbox."+name,localpath,path}) hold_directory(dir,true);
    return 0;
}

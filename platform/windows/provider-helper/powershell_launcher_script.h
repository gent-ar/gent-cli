#pragma once
#include <string>
#include <vector>
#include <stdexcept>
namespace contained_ps {
inline std::wstring quote(const std::wstring& text) {
    std::wstring out=L"\"";size_t slashes=0;
    for(wchar_t c:text){if(c==L'\\'){++slashes;continue;}
        if(c==L'"'){out.append(slashes*2+1,L'\\');out+=c;}
        else{out.append(slashes,L'\\');out+=c;}slashes=0;}
    out.append(slashes*2,L'\\');out+=L'"';return out;
}
inline std::wstring literal(const std::wstring& value) {
    std::wstring out=L"'";for(wchar_t c:value){out+=c;if(c==L'\'')out+=c;}return out+L"'";
}
inline bool equal(const std::wstring& a,const wchar_t* b){return _wcsicmp(a.c_str(),b)==0;}
inline bool contains(const std::wstring& parent,const std::wstring& child) {
    return child.size()>=parent.size()&&_wcsnicmp(parent.c_str(),child.c_str(),parent.size())==0&&
        (child.size()==parent.size()||child[parent.size()]==L'\\');
}
inline std::wstring dos(std::wstring path) {
    if(path.rfind(L"\\\\?\\UNC\\",0)==0)return L"\\\\"+path.substr(8);
    if(path.rfind(L"\\\\?\\",0)==0)return path.substr(4);
    return path;
}
inline std::vector<std::wstring> arguments(std::vector<std::wstring> args,
    const std::wstring& root,const std::wstring& cwd) {
    if(!contains(root,cwd))throw std::runtime_error("contained-shell-cwd-outside-workspace");
    if(args.size()==1&&(equal(args[0],L"--version")||equal(args[0],L"-Version")||
        equal(args[0],L"--help")||equal(args[0],L"-Help")||equal(args[0],L"-?")))return args;
    size_t command=args.size();
    for(size_t i=0;i<args.size();++i){
        if(equal(args[i],L"-Command")||equal(args[i],L"-c")){command=i;break;}
        // Claude's documented PowerShell argv changes execution policy only in this process.
        if(equal(args[i],L"-ExecutionPolicy")) {
            if(i+1>=args.size()||!equal(args[i+1],L"Bypass"))throw std::runtime_error("contained-shell-execution-policy");
            ++i;continue;
        }
        if(!equal(args[i],L"-NoLogo")&&!equal(args[i],L"-NoProfile")&&!equal(args[i],L"-NonInteractive"))
            throw std::runtime_error("contained-shell-unsupported-argument");
    }
    if(command+2!=args.size())throw std::runtime_error("contained-shell-command-shape");
    auto relative=cwd.substr(root.size());if(!relative.empty()&&relative.front()==L'\\')relative.erase(0,1);
    const auto location=L"GentWorkspace:\\"+relative;
    const std::wstring bootstrap=
        L"$__gent_previous_erroraction=$ErrorActionPreference;try{$ErrorActionPreference='Stop';$__gent_provider=$ExecutionContext.SessionState.Provider.Get('FileSystem')[0];"
        L"$__gent_drive=[System.Management.Automation.PSDriveInfo]::new('GentWorkspace',$__gent_provider,"+
        literal(dos(root))+L",'Gent contained workspace',[System.Management.Automation.PSCredential]::Empty);"
        L"[void]$ExecutionContext.SessionState.Drive.New($__gent_drive,$null);"
        L"[void]$ExecutionContext.SessionState.Path.SetLocation("+literal(location)+L");"
        L"if(![String]::Equals($ExecutionContext.SessionState.Path.CurrentFileSystemLocation.ProviderPath.TrimEnd([char]92),"+
        literal(dos(cwd))+L",[StringComparison]::OrdinalIgnoreCase)){throw 'contained location mismatch'}"
        L"}catch{[Console]::Error.WriteLine('GENT-CONTAINED-SHELL-ERROR contained-shell-bootstrap');exit 126}"
        L"finally{$ErrorActionPreference=$__gent_previous_erroraction};\n";
    args[command+1]=bootstrap+L". {\n"+args[command+1]+L"\n}\n";
    return args;
}
}

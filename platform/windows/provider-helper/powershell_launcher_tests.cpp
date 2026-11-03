#include <windows.h>
#include <shellapi.h>
#include <cstdio>
#include <stdexcept>
#include "powershell_launcher_script.h"
static void check(bool value){if(!value)throw std::runtime_error("launcher-test-failed");}
int wmain(int argc,wchar_t** argv) {
    if(argc==3&&std::wstring(argv[1])==L"--emit-init-failure") {
        using namespace contained_ps;const std::wstring root=argv[2];
        auto generated=arguments({L"-Command",L"Set-Content -LiteralPath 'UNEXPECTED-USER-BODY.txt' -Value 'bad'"},root,root);
        const auto script=L"$provider=$ExecutionContext.SessionState.Provider.Get('FileSystem')[0];$drive=[Management.Automation.PSDriveInfo]::new('GentWorkspace',$provider,"+literal(dos(root))+L",'collision control',[Management.Automation.PSCredential]::Empty);[void]$ExecutionContext.SessionState.Drive.New($drive,$null);"+generated.back();
        const int n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,script.data(),static_cast<int>(script.size()),nullptr,0,nullptr,nullptr);check(n>0);std::string utf8(static_cast<size_t>(n),'\0');
        check(WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,script.data(),static_cast<int>(script.size()),utf8.data(),n,nullptr,nullptr)==n);std::fwrite(utf8.data(),1,utf8.size(),stdout);return 0;
    }
    check(argc==1);
    using namespace contained_ps;
    const std::wstring root=L"\\\\?\\C:\\workspace space '\u4e2d";
    const auto args=arguments({L"-NoLogo",L"-NoProfile",L"-Command",L"param(); 'quoted \u00e9'; exit 7 #tail"},root,root+L"\\sub");
    check(args.size()==4&&args[0]==L"-NoLogo"&&args[1]==L"-NoProfile"&&args[2]==L"-Command");
    check(args[3].find(L"param(); 'quoted \u00e9'; exit 7 #tail\n}\n")!=std::wstring::npos);
    check(args[3].find(L"'C:\\workspace space ''\u4e2d'")!=std::wstring::npos);
    check(args[3].find(L"'GentWorkspace:\\sub'")!=std::wstring::npos);
    bool rejected=false;try{arguments({L"-Command",L"ok"},L"C:\\work",L"C:\\work-foreign");}catch(...){rejected=true;}check(rejected);
    rejected=false;try{arguments({L"-File",L"file.ps1"},root,root);}catch(...){rejected=true;}check(rejected);
    rejected=false;try{arguments({L"-Command",L"body",L"extra"},root,root);}catch(...){rejected=true;}check(rejected);
    const auto claude=arguments({L"-NoProfile",L"-NonInteractive",L"-ExecutionPolicy",L"Bypass",L"-Command",L"'claude marker'"},root,root);
    check(claude.size()==6&&claude[2]==L"-ExecutionPolicy"&&claude[3]==L"Bypass");
    check(claude.back().find(L"'claude marker'")!=std::wstring::npos);
    for(const auto& invalid:std::vector<std::vector<std::wstring>>{{L"-ExecutionPolicy"},{L"-ExecutionPolicy",L"Unrestricted",L"-Command",L"bad"}}) {
        rejected=false;try{arguments(invalid,root,root);}catch(...){rejected=true;}check(rejected);
    }
    check(arguments({L"--version"},root,root)==std::vector<std::wstring>{L"--version"});
    for(const auto& value:std::vector<std::wstring>{L"",L"plain",L"has space",L"quote\"slash\\",L"trailing \\",L"\u00e9\u4e2d"}) {
        int count=0;auto* parsed=CommandLineToArgvW((L"probe.exe "+quote(value)).c_str(),&count);
        check(parsed&&count==2&&parsed[1]==value);LocalFree(parsed);
    }
    std::puts("contained-powershell argument/bootstrap tests PASS");return 0;
}

#include <windows.h>
#include <cstdio>
#include <stdexcept>
#include "command_broker_protocol.h"
#include "command_environment.h"
static void check(bool value,const char* label){if(!value){std::printf("command-broker-test FAIL %s\n",label);std::exit(1);}}
static bool rejects(const std::vector<unsigned char>& body) {
    try{gent_command::decode(body);return false;}catch(const std::exception&){return true;}
}
static std::wstring block_of(const std::vector<wchar_t>& block){return std::wstring(block.data(),block.size());}
static void protocol_round_trip() {
    gent_command::Request request;request.target=gent_command::Target::PowerShell;request.stdio[1]=7;
    request.cwd=L"C:\\work";request.command=L"-c \"x y\"";request.environment=std::wstring(L"A=1\0B=2\0",8);
    const auto frame=gent_command::encode(request);
    const std::vector<unsigned char> body(frame.begin()+4,frame.end());
    const auto decoded=gent_command::decode(body);
    check(decoded.target==request.target&&decoded.cwd==request.cwd&&decoded.command==request.command&&decoded.environment==request.environment&&decoded.stdio[1]==7,"round-trip");
    check(rejects({}),"empty-frame");
    auto truncated=body;truncated.pop_back();check(rejects(truncated),"truncated-frame");
    auto wrong_target=body;wrong_target[4]=9;check(rejects(wrong_target),"unknown-target");
    auto wrong_magic=body;wrong_magic[0]^=1;check(rejects(wrong_magic),"magic");
}
static void command_tail_preserves_raw_arguments() {
    check(gent_command::command_tail(L"\"C:\\a b\\bash.exe\" -c \"echo \\\"x\\\"\"")==L"-c \"echo \\\"x\\\"\"","quoted-arg0");
    check(gent_command::command_tail(L"cmd.exe  /d /s /c \"exit 3\"")==L"/d /s /c \"exit 3\"","bare-arg0");
    check(gent_command::command_tail(L"bash.exe").empty(),"no-arguments");
}
static void environment_never_carries_gateway_authority() {
    gent_command::EnvironmentPolicy policy;policy.shim_directory=L"C:\\rt\\powershell-launcher";policy.windows_directory=L"C:\\Windows";
    policy.bearer=std::wstring(64,L'a');policy.gateway_origin=L"http://127.0.0.1:4242";policy.temp_root=L"C:\\s\\tmp";policy.appdata=L"C:\\s\\home\\appdata";
    policy.localappdata=L"C:\\s\\tmp\\command-localappdata";policy.real_shell_directories={L"C:\\rt\\git\\usr\\bin",L"C:\\rt\\powershell"};policy.tool_directories={L"C:\\tools\\git",L"C:\\extra"};policy.git_ceiling=L"C:\\parent";policy.bash_image=L"C:\\rt\\git\\usr\\bin\\bash.exe";policy.powershell_image=L"C:\\rt\\powershell\\pwsh.exe";
    std::wstring input;
    for(const auto& entry:std::vector<std::wstring>{L"ANTHROPIC_AUTH_TOKEN=x",L"anthropic_base_url=x",L"OPENAI_API_KEY=x",L"GENT_PROVIDER_GATEWAY_TOKEN=x",L"CUSTOM_OPENAI_API_KEY=x",L"CLAURST_HOME=x",L"LLAMA_CPP_HOST=x",
        L"HTTP_PROXY=x",L"https_proxy=x",L"ALL_PROXY=x",L"NO_PROXY=x",L"node_options=--max-old-space-size=64",L"GIT_CONFIG_COUNT=1",L"GIT_CONFIG_KEY_0=user.name",L"GIT_CONFIG_VALUE_0=kept",L"STOLEN="+policy.bearer,L"CLAUDE_CODE_MESSAGING_TOKEN=x",L"github_token=x",L"SOME_API_KEY=x",L"VENDOR_SECRET=x",L"TOKENIZER=kept",L"COPIED=http://127.0.0.1:4242/v1",L"KEEP=yes",L"TEMP=C:\\old",L"LOCALAPPDATA=C:\\old",L"PATHEXT=.EXE",L"SHELL=C:\\rt\\powershell-launcher\\bash.exe",L"ANY=C:\\rt\\powershell-launcher\\pwsh.exe",L"ComSpec=C:\\rt\\powershell-launcher\\cmd.exe",L"Path=C:\\rt\\powershell-launcher\\;C:\\extra"}){input+=entry;input.push_back(0);}
    const auto output=block_of(gent_command::command_environment(input,policy));
    for(const auto* secret:{L"ANTHROPIC",L"anthropic",L"OPENAI",L"GENT_",L"CLAURST",L"LLAMA",L"PROXY",L"proxy",L"STOLEN",L"COPIED",L"MESSAGING",L"github_token",L"SOME_API",L"VENDOR",L"4242",L"powershell-launcher",L"C:\\old"})check(output.find(secret)==std::wstring::npos,"secret-removed");
    check(output.find(L"ComSpec=C:\\Windows\\System32\\cmd.exe")!=std::wstring::npos,"real-comspec");
    check(output.find(L"PATHEXT=.COM;.EXE;.BAT;.CMD")!=std::wstring::npos,"command-pathext");
    check(output.find(L"SHELL=C:\\rt\\git\\usr\\bin\\bash.exe")!=std::wstring::npos&&output.find(L"ANY=C:\\rt\\powershell\\pwsh.exe")!=std::wstring::npos,"real-images");
    check(output.find(L"GIT_CEILING_DIRECTORIES=C:\\parent")!=std::wstring::npos,"git-ceiling");
    check(output.find(L"TOKENIZER=kept")!=std::wstring::npos,"suffix-only");
    check(output.find(L"NODE_OPTIONS=--max-old-space-size=64 --preserve-symlinks --preserve-symlinks-main")!=std::wstring::npos,"node-options-merged");
    check(output.find(L"node_options=")==std::wstring::npos,"node-options-single");
    check(output.find(L"GIT_CONFIG_COUNT=3")!=std::wstring::npos&&output.find(L"GIT_CONFIG_KEY_0=user.name")!=std::wstring::npos,"git-config-merged");
    check(output.find(L"GIT_CONFIG_KEY_1=url.gent::https://.insteadOf")!=std::wstring::npos&&output.find(L"GIT_CONFIG_VALUE_1=https://")!=std::wstring::npos,"git-https-routed");
    check(output.find(L"GIT_CONFIG_KEY_2=url.gent::http://.insteadOf")!=std::wstring::npos,"git-http-routed");
    check(output.find(L"KEEP=yes")!=std::wstring::npos,"ordinary-kept");
    check(output.find(L"PATH=C:\\rt\\git\\usr\\bin;C:\\rt\\powershell;C:\\extra;C:\\Windows\\System32;C:\\Windows;C:\\tools\\git")!=std::wstring::npos,"path-rewrite");
    check(output.find(L"TEMP=C:\\s\\tmp")!=std::wstring::npos&&output.find(L"LOCALAPPDATA=C:\\s\\tmp\\command-localappdata")!=std::wstring::npos,"private-directories");
}
static void working_directory_stays_in_granted_roots() {
    const std::vector<std::wstring> roots{L"C:\\work",L"C:\\Windows"};
    check(gent_command::command_directory_allowed(L"C:\\work\\sub",roots),"inside");
    check(gent_command::command_directory_allowed(L"c:\\WORK",roots),"case");
    check(gent_command::command_directory_allowed(L"C:\\work\\..hidden",roots),"dotted-name");
    check(!gent_command::command_directory_allowed(L"C:\\workspace",roots),"sibling");
    check(!gent_command::command_directory_allowed(L"C:\\work\\..\\other",roots),"traversal");
    check(!gent_command::command_directory_allowed(L"\\\\server\\share",roots),"unc");
    check(!gent_command::command_directory_allowed(L"\\\\?\\C:\\work",roots),"verbatim");
}
int main() {
    protocol_round_trip();command_tail_preserves_raw_arguments();environment_never_carries_gateway_authority();working_directory_stays_in_granted_roots();
    std::puts("command-broker-test PASS");return 0;
}

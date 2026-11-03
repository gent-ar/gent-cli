#pragma once
#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>
namespace gent_command {
inline constexpr const wchar_t* pipe_leaf=L"gent-command-broker";
inline constexpr uint32_t request_magic=0x31424347;
inline constexpr size_t max_frame_bytes=4u*1024u*1024u;
inline constexpr size_t max_command_chars=32000;
enum class Target:uint32_t{Bash=1,PowerShell=2,Cmd=3};
enum class ReplyKind:uint32_t{Started=1,Exit=2,Rejected=3};
struct ReplyHead{uint32_t kind=0;uint32_t value=0;};
struct RequestHead{uint32_t magic=0,target=0;uint64_t stdio[3]{};uint32_t cwd_chars=0,command_chars=0,environment_chars=0,reserved=0;};
struct Request{Target target=Target::Bash;uint64_t stdio[3]{};std::wstring cwd,command,environment;};
inline std::vector<unsigned char> encode(const Request& request) {
    RequestHead head;head.magic=request_magic;head.target=static_cast<uint32_t>(request.target);
    std::memcpy(head.stdio,request.stdio,sizeof(head.stdio));
    head.cwd_chars=static_cast<uint32_t>(request.cwd.size());head.command_chars=static_cast<uint32_t>(request.command.size());
    head.environment_chars=static_cast<uint32_t>(request.environment.size());
    const size_t body=(request.cwd.size()+request.command.size()+request.environment.size())*sizeof(wchar_t);
    if(sizeof(head)+body>max_frame_bytes)throw std::runtime_error("command-request-bound");
    std::vector<unsigned char> frame(sizeof(uint32_t)+sizeof(head)+body);
    const auto total=static_cast<uint32_t>(sizeof(head)+body);std::memcpy(frame.data(),&total,sizeof(total));
    auto* out=frame.data()+sizeof(uint32_t);std::memcpy(out,&head,sizeof(head));out+=sizeof(head);
    for(const auto* text:{&request.cwd,&request.command,&request.environment}){
        if(!text->empty())std::memcpy(out,text->data(),text->size()*sizeof(wchar_t));
        out+=text->size()*sizeof(wchar_t);
    }
    return frame;
}
inline Request decode(const std::vector<unsigned char>& body) {
    RequestHead head;
    if(body.size()<sizeof(head)||body.size()>max_frame_bytes)throw std::runtime_error("command-request-size");
    std::memcpy(&head,body.data(),sizeof(head));
    const uint64_t chars=uint64_t(head.cwd_chars)+head.command_chars+head.environment_chars;
    if(head.magic!=request_magic||chars*sizeof(wchar_t)!=body.size()-sizeof(head)||head.cwd_chars==0||head.command_chars>max_command_chars)throw std::runtime_error("command-request-shape");
    if(head.target<1||head.target>3)throw std::runtime_error("command-target");
    Request request;request.target=static_cast<Target>(head.target);std::memcpy(request.stdio,head.stdio,sizeof(head.stdio));
    const auto* cursor=reinterpret_cast<const wchar_t*>(body.data()+sizeof(head));
    request.cwd.assign(cursor,head.cwd_chars);cursor+=head.cwd_chars;
    request.command.assign(cursor,head.command_chars);cursor+=head.command_chars;
    request.environment.assign(cursor,head.environment_chars);
    if(request.cwd.find(L'\0')!=std::wstring::npos||request.command.find(L'\0')!=std::wstring::npos)throw std::runtime_error("command-request-text");
    return request;
}
inline std::wstring command_tail(const std::wstring& line) {
    size_t i=0;
    if(!line.empty()&&line[0]==L'"'){i=line.find(L'"',1);i=i==std::wstring::npos?line.size():i+1;}
    else while(i<line.size()&&line[i]!=L' '&&line[i]!=L'\t')++i;
    while(i<line.size()&&(line[i]==L' '||line[i]==L'\t'))++i;
    return line.substr(i);
}
}

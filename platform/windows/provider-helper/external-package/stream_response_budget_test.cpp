#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <stdexcept>
#include <thread>
#include <vector>
#include <cstdio>
#include "packaged_http_forwarder.cpp"
#include "host_pipe_bridge_exchange.h"
static void need(bool v){if(!v)throw std::runtime_error("stream-budget-test-failed");}
struct Pair {
    SOCKET client=INVALID_SOCKET,server=INVALID_SOCKET;
    Pair(){
        SOCKET listener=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);need(listener!=INVALID_SOCKET);
        sockaddr_in address{};address.sin_family=AF_INET;address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
        need(bind(listener,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0);need(listen(listener,1)==0);
        int length=sizeof(address);need(getsockname(listener,reinterpret_cast<sockaddr*>(&address),&length)==0);
        client=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);need(client!=INVALID_SOCKET);int small=1024;
        need(setsockopt(client,SOL_SOCKET,SO_RCVBUF,reinterpret_cast<char*>(&small),sizeof(small))==0);
        need(connect(client,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0);
        server=accept(listener,nullptr,nullptr);closesocket(listener);need(server!=INVALID_SOCKET);
        need(setsockopt(server,SOL_SOCKET,SO_SNDBUF,reinterpret_cast<char*>(&small),sizeof(small))==0);
    }
    ~Pair(){if(client!=INVALID_SOCKET)closesocket(client);if(server!=INVALID_SOCKET)closesocket(server);}
};
struct Pipe {
    HANDLE host=nullptr,peer=nullptr;
    explicit Pipe(bool asynchronous=false){
        wchar_t name[128];swprintf_s(name,L"\\\\.\\pipe\\GentStreamTest-%lu-%llu",GetCurrentProcessId(),GetTickCount64());
        host=CreateNamedPipeW(name,PIPE_ACCESS_DUPLEX|(asynchronous?FILE_FLAG_OVERLAPPED:0),PIPE_TYPE_BYTE|PIPE_READMODE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,4096,4096,0,nullptr);
        need(host!=INVALID_HANDLE_VALUE);peer=CreateFileW(name,GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr);need(peer!=INVALID_HANDLE_VALUE);
        host_bridge::Handle ready(CreateEventW(nullptr,TRUE,FALSE,nullptr));OVERLAPPED connection{};connection.hEvent=ready.value;
        const BOOL connected=ConnectNamedPipe(host,asynchronous?&connection:nullptr);
        const DWORD error=connected?ERROR_SUCCESS:GetLastError();
        if(!connected&&error==ERROR_IO_PENDING){DWORD n=0;need(WaitForSingleObject(ready.value,1000)==WAIT_OBJECT_0);need(GetOverlappedResult(host,&connection,&n,FALSE)!=0);}
        else need(connected||error==ERROR_PIPE_CONNECTED);
    }
    ~Pipe(){if(peer&&peer!=INVALID_HANDLE_VALUE)CloseHandle(peer);if(host&&host!=INVALID_HANDLE_VALUE)CloseHandle(host);}
};
static void put(HANDLE pipe,const void* bytes,DWORD size){DWORD n=0;need(WriteFile(pipe,bytes,size,&n,nullptr)!=0&&n==size);}
static void packet(HANDLE pipe,const char* bytes,DWORD size){put(pipe,&size,4);put(pipe,bytes,size);}
static void budget_cases(){
    contained_stream::Budget first(100,300,120);need(first.deadline()==400);need(first.progress(250));need(first.deadline()==370);
    need(first.progress(360));need(first.deadline()==400);need(first.expired(400));need(!first.progress(400));
    contained_stream::Budget idle(100,300,120);need(idle.progress(100));need(idle.expired(220));need(!idle.progress(220));
    contained_stream::Budget overflow(UINT64_MAX-1,300,120);need(overflow.deadline()==UINT64_MAX);
}
static void slow_first_response(){
    Pair sockets;Pipe channel;host_bridge::Handle owner(CreateEventW(nullptr,TRUE,FALSE,nullptr));need(owner.value!=nullptr);
    bool acknowledged=false;
    std::thread producer([&]{
        Sleep(80);packet(channel.host,"slow-first",10);Sleep(10);packet(channel.host,"active",6);
        const uint32_t end=contained_protocol::end;put(channel.host,&end,4);
        const auto deadline=GetTickCount64()+1000;DWORD available=0;
        while(GetTickCount64()<deadline){if(!PeekNamedPipe(channel.host,nullptr,0,nullptr,&available,nullptr))break;if(available>=4)break;Sleep(1);}
        if(available>=4){uint32_t ack=0;DWORD n=0;acknowledged=ReadFile(channel.host,&ack,4,&n,nullptr)&&n==4&&ack==contained_protocol::ack;}
    });
    const bool ok=external_bridge::receive_response(sockets.server,channel.peer,owner.value,false,contained_stream::Budget(GetTickCount64(),300,40));
    producer.join();need(ok&&acknowledged);
    char bytes[16]{};DWORD timeout=500;setsockopt(sockets.client,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<char*>(&timeout),sizeof(timeout));
    int total=0;while(total<16){const int n=recv(sockets.client,bytes+total,16-total,0);need(n>0);total+=n;}
    need(std::string(bytes,16)=="slow-firstactive");
}
static void active_socket_cancel(){
    Pair sockets;host_bridge::Handle owner(CreateEventW(nullptr,TRUE,FALSE,nullptr));need(owner.value!=nullptr);
    u_long nonblocking=1;need(ioctlsocket(sockets.server,FIONBIO,&nonblocking)==0);
    std::vector<char> bytes(2*1024*1024,'x');std::thread cancel([&]{Sleep(25);SetEvent(owner.value);});
    const auto start=GetTickCount64();const bool sent=external_bridge::send_response_bounded(sockets.server,bytes.data(),bytes.size(),owner.value,contained_stream::Budget(start,1000,500));
    cancel.join();need(!sent&&GetTickCount64()-start<750);
}
static void active_io_cancel(){
    host_bridge::Handle stopped(CreateEventW(nullptr,TRUE,FALSE,nullptr)),peer(CreateEventW(nullptr,TRUE,FALSE,nullptr)),pending(CreateEventW(nullptr,TRUE,FALSE,nullptr));
    need(stopped.value&&peer.value&&pending.value);host_bridge::Io io{stopped.value,peer.value};Pipe channel(true);
    std::thread cancel([&]{Sleep(25);SetEvent(stopped.value);});const auto start=GetTickCount64();
    char byte=0;DWORD count=0;const bool completed=io.pipe(channel.host,&byte,1,count,false,contained_stream::Budget(start,1000,500).deadline());cancel.join();need(!completed&&GetTickCount64()-start<750);
}
static void blocked_first_frame_uses_refreshed_idle(){
    Pair sockets;Pipe channel(true);
    host_bridge::Handle stopped(CreateEventW(nullptr,TRUE,FALSE,nullptr)),peer(CreateEventW(nullptr,TRUE,FALSE,nullptr));need(stopped.value&&peer.value);
    std::vector<char> bytes(32768,'x');size_t sent=0;
    while(sent<bytes.size()){const int n=send(sockets.client,bytes.data()+sent,static_cast<int>(bytes.size()-sent),0);need(n>0);sent+=static_cast<size_t>(n);}
    const auto start=GetTickCount64();const host_bridge::Io io{stopped.value,peer.value};
    const bool result=host_bridge::response(channel.host,io,sockets.server,contained_stream::Budget(start,300,40));
    // The peer never consumes output. The first read is successful but the
    // first data-frame write is blocked; progress must replace initial300ms
    // with40ms idle for BOTH frame-header and frame-body writes.
    need(!result&&GetTickCount64()-start<220);
}
int main(){
    try{WSADATA data{};need(WSAStartup(MAKEWORD(2,2),&data)==0);budget_cases();slow_first_response();active_socket_cancel();active_io_cancel();blocked_first_frame_uses_refreshed_idle();WSACleanup();
        std::puts("stream budget: bounds + actual slow first data/ACK + active socket/overlapped cancellation PASS");return 0;
    }catch(const std::exception&){std::fputs("stream budget test failed\n",stderr);return 1;}
}

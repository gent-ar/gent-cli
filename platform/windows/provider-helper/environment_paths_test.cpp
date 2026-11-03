#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include "environment_paths.h"
#include <filesystem>
#include <iostream>
using namespace gent_environment_paths;
static int checks=0;
static void yes(bool x){require(x,"test-failed");++checks;}
template<class F> static void rejects(F f){bool failed=false;try{f();}catch(const std::exception&){failed=true;}yes(failed);}
int wmain(){
 wchar_t temp[32768];require(GetTempPathW(32768,temp)>0,"temp");
 const auto root=std::wstring(temp)+L"gent-env-proof-"+std::to_wstring(GetCurrentProcessId());
 auto io=[](const std::wstring& p){return gent_windows_paths::io_path(p);};
 require(CreateDirectoryW(io(root).c_str(),nullptr)!=0,"mkdir");
 try{
  auto good=root+L"\\space-\u00e9",wrong=root+L"\\wrong";
  require(CreateDirectoryW(io(good).c_str(),nullptr)!=0&&CreateDirectoryW(io(wrong).c_str(),nullptr)!=0,"dirs");
  auto open=[&](const std::wstring& p){return CreateFileW(io(p).c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS,nullptr);};
  {Handle h(open(good));auto out=ordinary(h.h);yes(out.rfind(L"\\\\?\\",0)!=0);yes(verified_candidate(h.h,out)==out);rejects([&]{verified_candidate(h.h,wrong);});
   auto link=root+L"\\link";require(CreateSymbolicLinkW(io(link).c_str(),io(good).c_str(),SYMBOLIC_LINK_FLAG_DIRECTORY|2)!=0,"symlink");rejects([&]{verified_candidate(h.h,link);});require(RemoveDirectoryW(io(link).c_str())!=0,"unlink");}
  auto deep=root;for(int i=0;i<12;++i){deep+=L"\\long-directory-component";require(CreateDirectoryW(io(deep).c_str(),nullptr)!=0,"deep");}
  {Handle h(open(deep));yes(ordinary(h.h).size()>260);}
  yes(project(L"\\\\?\\UNC\\server\\share\\folder")==L"\\\\server\\share\\folder");
  for(const auto p:{L"\\\\?\\GLOBALROOT\\Device\\x",L"C:\\x.\\y",L"C:\\NUL.txt",L"C:\\x\\..\\y",L"C:\\x?",L"\\\\.\\C:\\x"})rejects([&]{project(p);});
  std::filesystem::remove_all(io(root));std::cout<<checks<<" PASS\n";return 0;
 }catch(...){std::filesystem::remove_all(io(root));throw;}
}

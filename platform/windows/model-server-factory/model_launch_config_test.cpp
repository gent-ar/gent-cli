#include <cstdint>
#include <stdexcept>
#include <iostream>
#include <nlohmann/json.hpp>
static void need(bool v,const char* label){if(!v)throw std::runtime_error(label);}
#include "model_launch_config.h"
static unsigned passed=0;
static nlohmann::json valid(){return {{"schema",1},{"modelPath","X:\\model.gguf"},{"modelSha256",std::string(64,'a')},{"modelId","qa-model"},{"contextTokens",1024},{"maxOutputTokens",64},{"threads",1},{"reasoningBudget",0},{"enableThinking",false},{"templateKwargs",nlohmann::json::object()}};}
static void rejects(nlohmann::json v){bool bad=false;try{parse_model_launch_config(v.dump());}catch(const std::exception&){bad=true;}need(bad,"accepted-invalid-config");++passed;}
int main(){auto c=parse_model_launch_config(valid().dump());need(c.context==1024&&c.output==64&&c.model_id=="qa-model","valid-config");++passed;
 auto v=valid();v["gatewayUrl"]="http://host";rejects(v);v=valid();v.erase("modelSha256");rejects(v);v=valid();v["contextTokens"]=32769;rejects(v);v=valid();v["maxOutputTokens"]=1024;rejects(v);v=valid();v["maxOutputTokens"]=0;rejects(v);v=valid();v["threads"]=-1;rejects(v);v=valid();v["threads"]=9;rejects(v);v=valid();v["templatePath"]="X:\\template";rejects(v);v=valid();v["modelSha256"]=std::string(64,'A');rejects(v);v=valid();v["modelId"]="model\r\n";rejects(v);v=valid();v["templateKwargs"]=nlohmann::json::array();rejects(v);v=valid();v["schema"]=2;rejects(v);v=valid();v["contextTokens"]=std::numeric_limits<uint64_t>::max();rejects(v);std::cout<<passed<<" PASS\n";}

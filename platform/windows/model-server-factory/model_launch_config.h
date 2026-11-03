#pragma once
#include <nlohmann/json.hpp>
#include <set>
struct ModelLaunchConfig {
    std::string model_path,model_sha,model_id,template_path,template_sha;
    uint32_t context=1024,output=64,threads=1,reasoning_budget=0;
    bool thinking=false;
    nlohmann::json kwargs=nlohmann::json::object();
};
static ModelLaunchConfig parse_model_launch_config(const std::string& source){
    need(source.size()<=16384,"model-config-size");const auto v=nlohmann::json::parse(source);need(v.is_object(),"model-config-object");
    const std::set<std::string> required{"schema","modelPath","modelSha256","modelId","contextTokens","maxOutputTokens","threads","reasoningBudget","enableThinking","templateKwargs"};
    auto allowed=required;allowed.insert("templatePath");allowed.insert("templateSha256");for(auto it=v.begin();it!=v.end();++it)need(allowed.count(it.key())!=0,"model-config-field");for(const auto& key:required)need(v.contains(key),"model-config-missing");
    auto number=[&](const char* key,uint32_t maximum){need(v[key].is_number_integer(),"model-config-integer");const auto value=v[key].get<int64_t>();need(value>=0&&value<=maximum,"model-config-bound");return static_cast<uint32_t>(value);};
    need(number("schema",1)==1,"model-config-schema");ModelLaunchConfig out;out.model_path=v["modelPath"].get<std::string>();out.model_sha=v["modelSha256"].get<std::string>();out.model_id=v["modelId"].get<std::string>();
    auto digest=[](const std::string& value){return value.size()==64&&value.find_first_not_of("0123456789abcdef")==std::string::npos;};
    need(digest(out.model_sha),"model-config-digest");need(!out.model_id.empty()&&out.model_id.size()<=128&&out.model_id.find_first_not_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./")==std::string::npos,"model-config-id");
    out.context=number("contextTokens",32768);out.output=number("maxOutputTokens",24576);out.threads=number("threads",8);out.reasoning_budget=number("reasoningBudget",3072);need(out.context>=512&&out.output>0&&out.output<out.context&&out.threads>0,"model-config-limits");
    need(v["enableThinking"].is_boolean()&&v["templateKwargs"].is_object()&&v["templateKwargs"].size()<=32,"model-config-template");out.thinking=v["enableThinking"].get<bool>();out.kwargs=v["templateKwargs"];
    need(v.contains("templatePath")==v.contains("templateSha256"),"model-config-template-pair");if(v.contains("templatePath")){out.template_path=v["templatePath"].get<std::string>();out.template_sha=v["templateSha256"].get<std::string>();need(!out.template_path.empty()&&digest(out.template_sha),"model-config-template-pin");}
    return out;
}

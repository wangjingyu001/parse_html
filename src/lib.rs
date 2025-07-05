
use boa_engine::{Context, Source};
use scraper::{Html, Selector};
use regex::Regex;
use serde_json::{Value,Map};



fn get_script_list(html_str: &String) -> Vec<String> {
    let mut script_list: Vec<String> = Vec::new();

    let html = Html::parse_document(&html_str);
    let script_selector = Selector::parse("script").unwrap();
    
    for script_element in html.select(&script_selector) {
        let script_text = script_element.text().collect::<Vec<_>>().join("");
        // println!("{:?}", script_text);
        script_list.push(script_text);
    }
    script_list

    
}



fn deep_merge(result: &mut Map<String, Value>, item: Map<String, Value>) {
    for (key, value) in item {
        match value {
            // 如果当前值是对象，且result中已有同key的对象 -> 递归合并
            Value::Object(item_obj) => {
                if let Some(Value::Object(result_obj)) = result.get_mut(&key) {
                    deep_merge(result_obj, item_obj);
                } else {
                    // result中没有对应对象 -> 直接插入
                    result.insert(key, Value::Object(item_obj));
                }
            }
            // 其他类型（数组/字符串/数字等）-> 直接覆盖
            _ => {
                result.insert(key, value);
            }
        }
    }
}

pub fn run(html_str: &String) -> Result<Map<String, serde_json::Value>, Box<dyn std::error::Error>> {
    let script_list: Vec<String> = get_script_list(html_str);
    
    let mut result: Map<String, Value> = Map::new();
    
    let mut context = Context::default();
    context.eval(Source::from_bytes(r#"
        var window = this;
        var self = window;
        var top = window;
        var document = {};
        var location = {};
        var navigator = {
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/89.0.4389.82 Safari/537.36",
        };
    "#))?;

    for script_text in script_list {
        if let Ok(parsed_data) = serde_json::from_str::<serde_json::Value>(&script_text) {
            if let Some(obj) = parsed_data.as_object() {
                for (key, value) in obj {
                    result.insert(key.clone(), value.clone());
                }
            }
        }else{
            // context.eval(Source::from_bytes(script_text))?;
            match context.eval(Source::from_bytes(&script_text)) {
                Ok(_) => {} // 成功时不处理
                Err(_) => {
                    let json_list = extract_all_json(&script_text);
                    for json_part in json_list{
                        
                        deep_merge(&mut result, json_part);

                    }
                    continue; // 失败时继续下一个脚本
                }
            }
        }

    }
    match context.eval(Source::from_bytes(r#"
        var result = Object.entries(window).reduce((acc, [key, val]) => {
            // 如果当前 key 是要跳过的，直接返回 acc

            const valType = typeof val;

            // 如果值是函数，跳过
            if (valType === 'function' || valType === 'undefined') {
                return acc;
            }
            try{
                // 处理数组或对象
                if (val && (valType === 'object' || Array.isArray(val))) {
                    try{
                        JSON.stringify(val); // 测试是否可序列化
                        acc[key] = val; // 保留有效数据
                    }catch(e){}
                }
                // 处理字符串（仅当字符串是合法 JSON 时）
                else if (valType === 'string' ) {
                        try{
                        const parsedVal = JSON.parse(val); // 尝试解析
                        
                        acc[key] = parsedVal; 
                        
                        }catch(e){
                        if(val){
                            acc['assignment_data'][key] = val

                        }

                        }
                        
                        
                }else if ( valType === 'number'){
                    acc['assignment_data'][key] = val
                }

            } catch (e) {
                // 跳过不可序列化的值
            }

            return acc;
        }, {"assignment_data":{}})
        JSON.stringify(result)
    "#)){
        Ok(window_result) => {
         
            // 1. 获取 JS 字符串（`JsString`）
        let js_str = window_result.to_string(&mut context)?;
        
        // 2. 转换成 Rust 的 `String`
        let json_str = js_str.to_std_string_escaped();
        
        // 3. 解析成 `serde_json::Value`
        let parsed_value: Value = serde_json::from_str(&json_str)?;
        
        // 4. 插入到你的 `result`（假设是 `HashMap<String, JsonValue>`）
        result.insert("window_result".to_string(), parsed_value);

        }
        Err(e) => {
            eprintln!("Script evaluation failed: {:?}", e);
        }
    }


    Ok(result)

}

pub fn extract_all_json(script_text:&String) -> Vec<serde_json::Map<String, Value>>{
    let mut json_list: Vec<String> = Vec::new();
    let mut start: i32 = -1;
    let mut open_braces = 0;
    let mut open_brackets = 0;
    
    let re = Regex::new(r"[{}\[\]]").unwrap();
    for mat in re.find_iter(script_text){
        let ch = mat.as_str();
        match ch {
            "{" => {
                if open_braces == 0 && open_brackets == 0{
                    start = mat.start() as i32;
                }
                open_braces += 1;
            }
            "}" => {
                if open_braces > 0 {
                    open_braces -= 1
                }
                if open_braces == 0 && open_brackets == 0 && start != -1{
                    json_list.push(script_text[start as usize..mat.end()].to_string());
                    start = -1;
                }
            }
            "[" => {
                if open_braces == 0 && open_brackets == 0{
                    start = mat.start() as i32;
                }
                open_brackets += 1;
            }
            "]" => {
                if open_brackets > 0 {
                    open_brackets -= 1
                }
                if open_braces == 0 && open_brackets == 0 && start != -1{
                    json_list.push(script_text[start as usize..mat.end()].to_string());
                    start = -1;
                }
            }
            _ => {}
        }
    }
    let mut result: Vec<serde_json::Map<String, Value>> = Vec::new();

    if !json_list.is_empty(){
        for json_part in json_list{
            if let Ok(data) = serde_json::from_str::<Map<String, Value>>(&json_part) {
                result.push(data);
            }


        }
    }
    return result;


     
}



// fn main() -> JsResult<()> {
//     unsafe{env::set_var("RUST_BACKTRACE", "full");}
//     let html_str = read_html();
//     let result = run(&html_str);
//     println!("final_result {:?}", result);


//     Ok(())
 
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run() {
        let html_str = r#"
        <html>
            <body>
                <script>
                    var a = 1;
                    var b = 2;
                    var c = a + b;
                    console.log(c);
                </script>
            </body>
        </html>
        "#.to_string();
        let result = run(&html_str);
        // assert_eq!(result, 5);
        println!("result {:?}", result);
        
    }
}



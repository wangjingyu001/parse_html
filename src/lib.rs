use boa_engine::{Context, Source};
use scraper::{Html, Selector};
use regex::Regex;
use serde_json::{Value, Map};
use anyhow::Result;

fn get_script_list(html_str: &String) -> Vec<String> {
    let html = Html::parse_document(html_str);
    let script_selector = Selector::parse("script").unwrap();

    html.select(&script_selector)
        .map(|e| e.text().collect::<Vec<_>>().join(""))
        .collect()
}

fn deep_merge(result: &mut Map<String, Value>, item: Map<String, Value>) {
    for (key, value) in item {
        match value {
            Value::Object(item_obj) => {
                if let Some(Value::Object(result_obj)) = result.get_mut(&key) {
                    deep_merge(result_obj, item_obj);
                } else {
                    result.insert(key, Value::Object(item_obj));
                }
            }
            _ => {
                result.insert(key, value);
            }
        }
    }
}

pub fn extract_all_json(script_text: &String) -> Vec<Map<String, Value>> {
    let mut json_list = Vec::new();
    let mut start = -1;
    let mut open_braces = 0;
    let mut open_brackets = 0;
    let re = Regex::new(r"[{}\[\]]").unwrap();

    for mat in re.find_iter(script_text) {
        let ch = mat.as_str();
        match ch {
            "{" => {
                if open_braces == 0 && open_brackets == 0 {
                    start = mat.start() as i32;
                }
                open_braces += 1;
            }
            "}" => {
                if open_braces > 0 {
                    open_braces -= 1;
                }
                if open_braces == 0 && open_brackets == 0 && start != -1 {
                    json_list.push(script_text[start as usize..mat.end()].to_string());
                    start = -1;
                }
            }
            "[" => {
                if open_braces == 0 && open_brackets == 0 {
                    start = mat.start() as i32;
                }
                open_brackets += 1;
            }
            "]" => {
                if open_brackets > 0 {
                    open_brackets -= 1;
                }
                if open_braces == 0 && open_brackets == 0 && start != -1 {
                    json_list.push(script_text[start as usize..mat.end()].to_string());
                    start = -1;
                }
            }
            _ => {}
        }
    }

    let mut result = Vec::new();
    for json_part in json_list {
        if let Ok(data) = serde_json::from_str::<Map<String, Value>>(&json_part) {
            result.push(data);
        }
    }
    result
}

pub fn run(html_str: &String) -> Result<Map<String, serde_json::Value>> {
    let script_list = get_script_list(html_str);

    let sandbox_script = r#"
        var window = this;
        var self = window;
        var top = window;
        var document = {};
        var location = {};
        var navigator = {
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/89.0.4389.82 Safari/537.36",
        };
    "#;

    let script_list = script_list.clone();

    let handle = std::thread::Builder::new()
        .name("boa_eval_thread".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<Map<String, Value>> {
            let mut result: Map<String, Value> = Map::new();
            let mut context = Context::default();

            match context.eval(Source::from_bytes(sandbox_script)){
                Ok(_) => println!("✅ JS初始化成功"),
                Err(e) => {
                    eprintln!("❌ JS初始化失败");
                   
                }   
            };

            for (index, script_text) in script_list.iter().enumerate() {
                let now = std::time::Instant::now();
                println!("执行第 {} 个 script", index);

                if let Ok(parsed_data) = serde_json::from_str::<Value>(script_text) {
                    if let Some(obj) = parsed_data.as_object() {
                        for (key, value) in obj {
                            result.insert(key.clone(), value.clone());
                        }
                    }
                    continue;
                }

                match context.eval(Source::from_bytes(script_text)) {
                    Ok(_) => println!("✅ JS执行完成 index {} eval耗时: {:?}", index,now.elapsed()),
                    Err(e) => {
                        eprintln!("❌ JS执行出错 index {}: {:?} eval耗时: {:?}", index, e, now.elapsed());
                        let json_list = extract_all_json(script_text);
                        // println!("json_list {:?}", json_list);
                        for json_part in json_list {
                            deep_merge(&mut result, json_part);
                        }
                    }
                }
            }

            match context.eval(Source::from_bytes(r#"
                function safeExtract(obj, visited = new WeakSet(), depth = 0, maxDepth = 3, maxPropsPerObject = 5000000) {
    if (obj === null || typeof obj !== 'object') return obj;
    if (visited.has(obj)) return;
    if (depth > maxDepth) return;

    visited.add(obj);
    const result = {};
    let count = 0;

    const props = Object.keys(obj);  
    for (const key of props) {
        if (count >= maxPropsPerObject) {
            result['__truncated__'] = `Only first ${maxPropsPerObject} props extracted.`;
            break;
        }

        try {
            const value = obj[key];
            const type = typeof value;

            if (
                type === 'function' ||
                type === 'symbol' ||
                type === 'undefined' ||
                value === window
            ) {
                continue;
            }

            if (type === 'object') {
                const extracted = safeExtract(value, visited, depth + 1, maxDepth, maxPropsPerObject);
                if (extracted !== undefined) {
                    result[key] = extracted;
                    count++;
                }
            } else {
                result[key] = value;
                count++;
            }
        } catch (e) {
            result[key] = `[Error: ${e.message}]`;
            count++;
        }
    }

    return result;
};
JSON.stringify(safeExtract(window))
                 
            "#)){
                Ok(window_result) => {
                    // println!("window_result {:?}", window_result.display());
                    let now = std::time::Instant::now();
                    let js_str = window_result.to_string(&mut context).unwrap();
                    let json_str = js_str.to_std_string_escaped();
                    // let json_str = window_result.display().to_string();

                    let parsed_value: Value = serde_json::from_str(&json_str)?;
                    result.insert("window_result".to_string(), parsed_value);
                    println!("window_result eval耗时: {:?}", now.elapsed());

                }
                Err(e) => {
            eprintln!("Script evaluation failed: {:?}", e);
        }
            };

            

            Ok(result)
        })
        .unwrap();

    let thread_result = handle.join().unwrap()?;
    Ok(thread_result)
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
    use std::fs;
    use std::time::Instant;
    use std::fs::File;
    use std::io::prelude::*;
    use serde_json::Value;
    #[test] 
    fn test_run() {
        let html_str = match fs::read_to_string(r"C:\Users\Admin\PycharmProjects\my_js_parser性能测试\youtube.html") {
            Ok(data) => data,
            Err(e) => {
                eprintln!("❌ 读取文件失败: {}", e);
                return;
            }
        };
        // let result = run(&html_str);

        let start_time = Instant::now();
   
        // for _ in 1..=10 {
        
            let result = run(&html_str).unwrap();
            
    
        // }
        let end_time = Instant::now();
        let duration = end_time - start_time;

        // println!("结果 {:?}", result);
        let file = File::create("result.json").unwrap(); // 创建或覆盖文件
        serde_json::to_writer_pretty(file, &result).unwrap(); // 格式化写入 JSON
        println!("10次执行耗时: {:?}", duration);
 
        // assert_eq!(result, 5);
        // println!("result {:?}", result);
        
    }
}



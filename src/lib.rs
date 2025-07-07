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
                    Ok(_) => println!("✅ JS执行完成 index {}", index),
                    Err(e) => {
                        eprintln!("❌ JS执行出错 index {}: {:?}", index, e);
                        let json_list = extract_all_json(script_text);
                        for json_part in json_list {
                            deep_merge(&mut result, json_part);
                        }
                    }
                }
            }

            match context.eval(Source::from_bytes(r#"
                var result = Object.entries(window).reduce((acc, [key, val]) => {
                    const valType = typeof val;
                    if (valType === 'function' || valType === 'undefined') return acc;
                    try {
                        if (val && (valType === 'object' || Array.isArray(val))) {
                            try { JSON.stringify(val); acc[key] = val; } catch(e){}
                        } else if (valType === 'string') {
                            try { acc[key] = JSON.parse(val); }
                            catch(e){ if (val) acc.assignment_data[key] = val; }
                        } else if (valType === 'number') {
                            acc.assignment_data[key] = val;
                        }
                    } catch(e) {}
                    return acc;
                }, {assignment_data:{}})
                JSON.stringify(result)
            "#)){
                Ok(window_result) => {
                    let js_str = window_result.to_string(&mut context).unwrap();
                    let json_str = js_str.to_std_string_escaped();
                    let parsed_value: Value = serde_json::from_str(&json_str)?;
                    result.insert("window_result".to_string(), parsed_value);
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

    #[test]
    fn test_run() {
        let html_str = r#"
        <script>
            var a = 1;
            var b = 2;
            var c = a + b;
            var d = {
                "e": 3,
                "f": 4
            };
            var g = [5, 6, 7];
            var h = "hello world";
            var i = null;
            var j = undefined;
            var k = function() {
                console.log("hello");
            };
        </script>

        "#.to_string();
        let result = run(&html_str);
        // assert_eq!(result, 5);
        println!("result {:?}", result);
        
    }
}



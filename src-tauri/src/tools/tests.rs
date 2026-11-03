use super::file::*;
use super::helpers::*;
use super::subagent::{build_briefing_packet, extract_compact_summary, save_subtask_knowledge_artifact, save_subtask_report_artifact};
use super::types::*;
use super::web::*;
use serde_json::json;

    #[test]
    fn test_glob_regex() {
        let re = regex::Regex::new(&glob_to_regex("src/**/*.rs")).unwrap();
        assert!(re.is_match("src/a/b.rs"));
        assert!(re.is_match("src/main.rs"));
        assert!(!re.is_match("other/main.rs"));

        let re2 = regex::Regex::new(&glob_to_regex("*.json")).unwrap();
        assert!(re2.is_match("package.json"));
        assert!(!re2.is_match("a/package.json"));
    }

    #[test]
    fn test_high_danger() {
        assert!(is_high_danger("rm -rf /"));
        assert!(is_high_danger("rm -r /tmp"));
        assert!(!is_high_danger("rm -f test.txt"));
        assert!(!is_high_danger("rm file.txt"));
        assert!(is_high_danger("git reset --hard HEAD~1"));
        assert!(is_high_danger("del /s /q *.tmp"));
        assert!(is_high_danger("del /q /s *.tmp"));
        assert!(!is_high_danger("del /q tmp-verify.ps1"));
        assert!(!is_high_danger("cmd /c \"del /q tmp-verify.ps1\" 2>&1"));
        assert!(!is_high_danger("del tmp-verify.ps1"));
        assert!(is_high_danger("rd /s /q temp"));
        assert!(!is_high_danger("rd temp"));
        assert!(!is_high_danger("npm run build"));
        assert!(!is_high_danger("cargo test"));
    }

    #[test]
    fn test_extract_outline_rust() {
        let code = r#"
// Some comment
pub struct User {
    pub name: String,
}

impl User {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

async fn fetch_data() -> Result<(), ()> {
    Ok(())
}
"#;
        let outline = extract_outline(code, "rs");
        assert!(outline.contains("pub struct User"));
        assert!(outline.contains("impl User"));
        assert!(outline.contains("pub fn new"));
        assert!(outline.contains("async fn fetch_data"));
        assert!(!outline.contains("Some comment"));
        assert!(!outline.contains("pub name: String"));
    }

    #[test]
    fn test_extract_outline_ts() {
        let code = r#"
export interface Item {
  id: string;
}

export const LIST = [1, 2];

export function getItem(): Item {
  return { id: "1" };
}

const handleClick = async () => {
  console.log("clicked");
};
"#;
        let outline = extract_outline(code, "ts");
        assert!(outline.contains("export interface Item"));
        assert!(outline.contains("export const LIST"));
        assert!(outline.contains("export function getItem"));
        assert!(outline.contains("const handleClick"));
    }

    #[test]
    fn test_inside_workspace() {
        let temp = std::env::temp_dir();
        let ctx = ToolCtx {
            workspace: temp.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
            message_id: None,
            proxy_url: None,
        };

        // 工作区内已存在目录
        assert!(inside_workspace(&ctx, "."));
        // 工作区内新建文件（不存在）
        assert!(inside_workspace(&ctx, "non_existent_file_abc123.txt"));
        // 工作区内多层不存在子路径
        assert!(inside_workspace(&ctx, "a/b/c/new_file.txt"));
        // 尝试越界
        assert!(!inside_workspace(&ctx, "../../outside_something_abc123"));
    }

    #[test]
    fn test_args_helpers() {
        let json_data = json!({
            "num": 123,
            "str_num": "456",
            "invalid_num": "not_a_num",
            "bool_val": true,
            "str_bool_true": "true",
            "str_bool_false": "FALSE",
            "num_bool_1": 1,
            "num_bool_0": 0
        });

        assert_eq!(get_u64_arg(&json_data, "num", 0), 123);
        assert_eq!(get_u64_arg(&json_data, "str_num", 0), 456);
        assert_eq!(get_u64_arg(&json_data, "invalid_num", 99), 99);
        assert_eq!(get_u64_arg(&json_data, "non_existent", 42), 42);

        assert_eq!(get_bool_arg(&json_data, "bool_val", false), true);
        assert_eq!(get_bool_arg(&json_data, "str_bool_true", false), true);
        assert_eq!(get_bool_arg(&json_data, "str_bool_false", true), false);
        assert_eq!(get_bool_arg(&json_data, "num_bool_1", false), true);
        assert_eq!(get_bool_arg(&json_data, "num_bool_0", true), false);
        assert_eq!(get_bool_arg(&json_data, "non_existent", true), true);
    }

    #[test]
    fn test_resolve_slash_trim() {
        let temp = std::env::temp_dir();
        let ctx = ToolCtx {
            workspace: temp.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
            message_id: None,
            proxy_url: None,
        };

        let resolved_normal = resolve(&ctx, "src/main.rs");
        let resolved_slash = resolve(&ctx, "/src/main.rs");
        let resolved_backslash = resolve(&ctx, "\\src\\main.rs");

        assert_eq!(resolved_normal, temp.join("src/main.rs"));
        assert_eq!(resolved_slash, temp.join("src/main.rs"));
        assert_eq!(resolved_backslash, temp.join("src\\main.rs"));
    }

    #[test]
    fn test_glob_to_regex_braces() {
        let re_str = glob_to_regex("*.{js,ts}");
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("index.js"));
        assert!(re.is_match("index.ts"));
        assert!(!re.is_match("index.rs"));

        let re_nested = glob_to_regex("src/**/*.{java,kt}");
        let re2 = regex::Regex::new(&re_nested).unwrap();
        assert!(re2.is_match("src/models/User.java"));
        assert!(re2.is_match("src/User.kt"));
        assert!(!re2.is_match("src/User.cpp"));
    }

    #[test]
    fn test_find_match_line_numbers() {
        let text = "alpha\nbeta\ngamma\nbeta\nomega";
        let lines = find_match_line_numbers(text, "beta");
        assert_eq!(lines, vec![2, 4]);
    }

    #[test]
    fn test_diagnose_edit_mismatch() {
        let text = "public class Hello {\n    void run() {\n        System.out.println(\"ok\");\n    }\n}";
        // 缩进不匹配
        let old = "  void run() {\n      System.out.println(\"ok\");\n  }";
        let diag = diagnose_edit_mismatch(text, old);
        assert!(diag.contains("相似行") || diag.contains("忽略缩进"));

        // 完全没有的
        let not_found = "class NonExistentFooBarBaz {}";
        let diag2 = diagnose_edit_mismatch(text, not_found);
        assert!(diag2.contains("系统已自动统一 CRLF/LF"));
    }

    #[tokio::test]
    async fn test_edit_file_crlf_preservation() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_edit_{}", uuid::Uuid::new_v4()));
        let _ = tokio::fs::create_dir_all(&temp_dir).await;
        let test_file = temp_dir.join("Test.java");

        // 写入 CRLF 格式原始文件
        let original_crlf = "public class Test {\r\n    private int a;\r\n}\r\n";
        tokio::fs::write(&test_file, original_crlf).await.unwrap();

        let ctx = ToolCtx {
            workspace: temp_dir.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
            message_id: None,
            proxy_url: None,
        };

        // LLM 发送 LF 换行的 old_string 与 new_string
        let args = json!({
            "path": "Test.java",
            "old_string": "    private int a;\n",
            "new_string": "    private int a;\n    private int b;\n"
        });

        let res = edit_file(&args, &ctx).await;
        assert!(res.is_ok(), "edit_file 应该成功匹配 LF 与 CRLF: {:?}", res.err());

        // 读取磁盘，验证仍为 CRLF
        let modified = tokio::fs::read_to_string(&test_file).await.unwrap();
        assert!(modified.contains("\r\n"), "应当保留文件原有的 CRLF 换行格式");
        assert!(modified.contains("private int b;"));

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[test]
    fn test_briefing_packet_and_artifact_summary() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let ws = std::env::temp_dir().join(format!("harness_test_briefing_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();

        let parent_sess = crate::store::create_session(&conn, &ws.to_string_lossy(), None, "主会话", "standard").unwrap();

        // 1. 测试简报生成（工程施工工人角色）
        let args = json!({
            "relevant_files": ["src/auth.rs", "src/token.rs"],
            "acceptance_criteria": "cargo test test_auth",
            "constraints": "严禁修改外部接口"
        });
        let packet = build_briefing_packet(&conn, &ws, &parent_sess.id, "后端工程师", "重构JWT", "实现Token无感刷新", &args);
        assert!(packet.contains("【任务协同委派简报】"));
        assert!(packet.contains("src/auth.rs"));
        assert!(packet.contains("src/token.rs"));
        assert!(packet.contains("cargo test test_auth"));
        assert!(packet.contains("严禁修改外部接口"));
        assert!(packet.contains("【施工工人准则】"));
        assert!(!packet.contains("mermaid"));

        // 1.1 测试架构调研分析角色分化简报
        let research_args = json!({
            "relevant_files": ["src/router.rs"],
        });
        let research_packet = build_briefing_packet(&conn, &ws, &parent_sess.id, "架构师", "路由协议调研", "摸清路由分发机制", &research_args);
        assert!(research_packet.contains("【架构调研/分析准则】"));
        assert!(research_packet.contains("mermaid"));

        // 2. 测试知识交接工件保存
        let knowledge_path = save_subtask_knowledge_artifact(&ws, "sub_123", "重构JWT", "后端工程师", &packet, &parent_sess.id).unwrap();
        assert!(knowledge_path.contains(".harness/subtasks/sub_123_knowledge.md"));
        let full_knowledge_path = ws.join(&knowledge_path);
        assert!(full_knowledge_path.exists());
        let knowledge_content = std::fs::read_to_string(&full_knowledge_path).unwrap();
        assert!(knowledge_content.contains("# 协同子任务知识交接书: 重构JWT"));
        assert!(knowledge_content.contains(&parent_sess.id));
        assert!(knowledge_content.contains("src/auth.rs"));

        // 3. 测试报告工件保存与紧凑摘要截断
        let touched = vec!["src/auth.rs".to_string()];
        let full_text = "A".repeat(800);
        let artifact_path = save_subtask_report_artifact(&ws, "sub_123", "测试任务", "测试角色", &full_text, &touched).unwrap();
        assert!(artifact_path.contains(".harness/subtasks/sub_123_report.md"));
        assert!(ws.join(&artifact_path).exists());

        let summary = extract_compact_summary(&full_text, 100);
        assert!(summary.len() < 250);
        assert!(summary.contains("内容已自动压缩提炼"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_html_to_markdown() {
        let sample_html = r#"
            <!DOCTYPE html>
            <html>
            <head><title>Test Page</title><style>.hidden { display: none; }</style></head>
            <body>
                <script>alert("evil");</script>
                <svg><path d="M0 0"/></svg>
                <h1>Article Title</h1>
                <p>Hello, this is a <a href="https://example.com">link</a> to example.</p>
                <p>Line with &quot;quotes&quot; and &amp; ampersand.</p>
            </body>
            </html>
        "#;
        let md = html_to_markdown(sample_html);
        assert!(!md.contains("alert"));
        assert!(!md.contains("<script"));
        assert!(!md.contains("<svg"));
        assert!(!md.contains(".hidden"));
        assert!(md.contains("# Article Title"));
        assert!(md.contains("[link](https://example.com)"));
        assert!(md.contains(r#""quotes" and & ampersand"#));
    }

    #[tokio::test]
    async fn test_core_tools_contain_call_extended_tool_and_catalog_filtering() {
        let registry = super::registry::ToolRegistry::new();
        let specs = registry.list_specs(false, false).await;
        let names: Vec<&str> = specs.iter().map(|s| s.name).collect();

        // 核心工具数量严格收敛
        assert_eq!(names.len(), CORE_TOOL_NAMES.len());
        assert!(names.contains(&"call_extended_tool"));
        assert!(names.contains(&"lookup_tool"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"create_plan"));
        assert!(names.contains(&"update_plan"));
        assert!(names.contains(&"todo"));

        // 扩展工具被收敛，不在顶层直接暴露
        assert!(!names.contains(&"browser_navigate"));
        assert!(!names.contains(&"browser_interact"));
        assert!(!names.contains(&"record_memory"));
    }

    #[tokio::test]
    async fn test_call_extended_tool_help() {
        let ws = std::env::temp_dir().join(format!("hm_test_cet_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&ws);
        let ctx = ToolCtx {
            workspace: ws.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(5),
            temp: None,
            host: None,
            event_id: None,
            message_id: None,
            proxy_url: None,
        };
        let on_partial = |_line: &str| {};
        let args = json!({
            "tool_name": "browser_navigate",
            "arguments": {
                "help": true
            }
        });
        let res = super::dispatch_static("call_extended_tool", &args, &ctx, &on_partial).await.unwrap();
        assert!(res.contains("browser_navigate"));
        assert!(res.contains("参数 Schema"));
        assert!(res.contains("Chromium"));

        // 测试原生 lookup_tool
        let lookup_args = json!({ "tool_name": "spawn_subagent" });
        let lookup_res = super::dispatch_static("lookup_tool", &lookup_args, &ctx, &on_partial).await.unwrap();
        assert!(lookup_res.contains("spawn_subagent"));
        assert!(lookup_res.contains("参数 Schema"));

        // 测试扩展工具执行失败时自动附加 Schema 纠错指引
        let bad_args = json!({
            "tool_name": "record_memory",
            "arguments": {}
        });
        let bad_res = super::dispatch_static("call_extended_tool", &bad_args, &ctx, &on_partial).await.unwrap_err();
        assert!(bad_res.contains("缺少 content 参数"));
        assert!(bad_res.contains("【工具 `record_memory` 参数规范与纠错指引】"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_subagent_blackboard_handover_mount_and_no_deliver_to_parent() {
        let ws = std::env::temp_dir().join(format!("hm_test_bb_{}", uuid::Uuid::new_v4()));
        let subtasks_dir = ws.join(".harness").join("subtasks");
        let _ = std::fs::create_dir_all(&subtasks_dir);

        let subagent_id = "test-sub-456";
        let knowledge_filename = format!("{}_knowledge.md", subagent_id.replace('-', "_"));
        let knowledge_file = subtasks_dir.join(&knowledge_filename);
        std::fs::write(&knowledge_file, "# 知识交接书内容\n- 目标：全面调研架构\n- 验收标准：完成报告").unwrap();

        let sub_session = crate::models::Session {
            id: subagent_id.into(),
            parent_session_id: Some("parent-123".into()),
            session_type: "subagent".into(),
            workspace_path: ws.to_string_lossy().to_string(),
            subagent_role: Some("架构调研员".into()),
            title: "调研任务".into(),
            ..Default::default()
        };

        // 验证 System Prompt 自动挂载黑板交接书
        let sys_prompt = crate::context::build_static_system_prompt(&sub_session, &[], "standard", None, None, None);
        assert!(sys_prompt.contains("## 📋 协同任务黑板交接书"));
        assert!(sys_prompt.contains("全面调研架构"));
        assert!(sys_prompt.contains("test_sub_456_report.md"));
        assert!(!sys_prompt.contains("deliver_to_parent"));

        // 验证工具列表中已无 deliver_to_parent 专用工具
        let specs = super::specs::tool_specs();
        assert!(!specs.iter().any(|s| s.name == "deliver_to_parent"));

        let _ = std::fs::remove_dir_all(&ws);
    }

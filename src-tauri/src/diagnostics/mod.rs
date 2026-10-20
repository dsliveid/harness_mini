pub mod build_stream;
pub mod overlay_sniffer;
pub mod types;

pub use build_stream::{probe_dev_server, DevServerStatus};
pub use overlay_sniffer::EXTRACT_ERROR_OVERLAYS_SCRIPT;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_report_rendering() {
        let mut report = DiagnosticReport::default();
        report.dom_overlays.push("[Webpack 编译错误]: Module build failed".to_string());
        report.runtime_errors.push(JsRuntimeError {
            name: "TypeError".to_string(),
            message: "Cannot read properties of undefined".to_string(),
            stack_trace: vec![SourceLocation {
                file: "medicalDisputeForm.vue".to_string(),
                line: 757,
                column: 33,
                function_name: Some("handleHospitalParticipantsChange".to_string()),
            }],
            raw_description: "TypeError: Cannot read properties of undefined\n  at handleHospitalParticipantsChange".to_string(),
            timestamp: 12345678,
        });
        report.network_errors.push(NetworkErrorItem {
            url: "http://localhost:8080/api/save".to_string(),
            method: "POST".to_string(),
            status: Some(500),
            failure_reason: "HTTP 500".to_string(),
            response_body_snippet: Some("{\"msg\":\"Internal Server Error\"}".to_string()),
        });

        assert!(report.has_errors());
        let md = report.to_markdown();
        assert!(md.contains("Webpack 编译错误"));
        assert!(md.contains("TypeError"));
        assert!(md.contains("medicalDisputeForm.vue:757:33"));
        assert!(md.contains("handleHospitalParticipantsChange"));
        assert!(md.contains("[HTTP 500] POST http://localhost:8080/api/save"));
    }
}


use std::path::PathBuf;

/// 自动探测系统中安装的 Chromium 内核浏览器（优先 Edge，其次 Chrome，再次系统默认 Chromium）
pub fn detect_chromium_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        // 1. 常见 Edge 路径
        let candidates = [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];

        for path in candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }

        // 2. 从 LOCALAPPDATA 探测 Edge / Chrome
        if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
            let edge_user = PathBuf::from(&local_app).join(r"Microsoft\Edge\Application\msedge.exe");
            if edge_user.exists() {
                return Ok(edge_user);
            }
            let chrome_user = PathBuf::from(&local_app).join(r"Google\Chrome\Application\chrome.exe");
            if chrome_user.exists() {
                return Ok(chrome_user);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/snap/bin/chromium",
        ];
        for path in candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for path in candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }
    }

    Err("未在系统中检测到支持的 Chromium 浏览器 (Edge 或 Chrome)，请确保已安装 Edge 或 Chrome".to_string())
}

use anyhow::Result;
use notify_rust::Notification;

/// 通知管理器
#[derive(Clone)]
pub struct NotificationManager {
    enabled: bool,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// 发送系统通知
    pub fn send(&self, title: &str, message: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        println!("🔔 {}: {}", title, message); // 先在控制台显示

        // 尝试发送系统通知
        match Notification::new()
            .summary(title)
            .body(message)
            .timeout(3000) // 3秒后消失
            .show()
        {
            Ok(_) => {}
            Err(e) => {
                // 如果系统通知失败，不要崩溃程序
                eprintln!("系统通知发送失败: {}", e);
            }
        }

        Ok(())
    }

    /// 启用/禁用通知
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 检查是否启用通知
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

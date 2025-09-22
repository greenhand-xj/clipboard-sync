mod clipboard;
mod network;
mod notification;

use anyhow::Result;
use clap::{Parser, Subcommand};
use clipboard::ClipboardManager;
use network::NetworkManager;
use notification::NotificationManager;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "clipboard-sync")]
#[command(about = "跨平台剪贴板同步工具")]
struct Cli {
    /// 设备名称
    #[arg(short, long, default_value = "我的设备")]
    name: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动同步服务（创建新网络）
    Start,
    /// 连接到现有设备
    Connect {
        /// 连接票据
        ticket: String,
    },
    /// 生成连接票据
    Ticket,
    /// 自动搜索其他设备
    Auto,
    /// 测试剪贴板功能
    Test,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 初始化剪贴板管理器
    let clipboard = ClipboardManager::new()?;

    match cli.command {
        Commands::Test => {
            test_clipboard(clipboard).await?;
        }
        Commands::Start => {
            let network = NetworkManager::new(cli.name.clone()).await?;
            run_sync_service(clipboard, network).await?;
        }
        Commands::Connect { ticket } => {
            let network = NetworkManager::new(cli.name.clone()).await?;
            connect_to_peer(clipboard, network, &ticket).await?;
        }
        Commands::Ticket => {
            let network = NetworkManager::new(cli.name.clone()).await?;
            let ticket = network.generate_ticket().await?;
            println!("连接票据:");
            println!("{}", ticket);
            println!("\n在其他设备上运行以下命令来连接:");
            println!("clipboard-sync connect {}", ticket);
        }
        Commands::Auto => {
            let network = NetworkManager::new(cli.name.clone()).await?;
            auto_connect(clipboard, network).await?;
        }
    }

    Ok(())
}

async fn auto_connect(clipboard: ClipboardManager, network: NetworkManager) -> Result<()> {
    let notifier = NotificationManager::new();

    println!("🔍 启动自动连接模式...");
    notifier.send("剪贴板同步", "自动搜索其他设备中...")?;

    // 设置消息处理器
    let mut message_receiver = network.setup_message_handler().await;

    // 启动网络监听任务
    // let network_clone = network.clone();
    // tokio::spawn(async move {
    //     if let Err(e) = network_clone.listen_for_connections().await {
    //         eprintln!("网络监听失败: {}", e);
    //     }
    // });

    // 启动自动发现任务
    let network_discovery = network.clone();
    tokio::spawn(async move {
        if let Err(e) = network_discovery.start_auto_discovery().await {
            eprintln!("自动发现失败: {}", e);
        }
    });

    // 启动消息处理任务
    let clipboard_clone = clipboard.clone();
    let notifier_clone = notifier.clone();
    tokio::spawn(async move {
        while let Some(message) = message_receiver.recv().await {
            println!(
                "收到剪贴板消息: {} (来自: {})",
                message.content, message.sender_id
            );

            // 根据消息类型更新本地剪贴板
            match &message.content {
                network::ClipboardContent::Text(text) => {
                    if let Err(e) = clipboard_clone.set_text(text) {
                        eprintln!("更新文本剪贴板失败: {}", e);
                    } else {
                        let preview = message.content.preview(50);
                        let _ = notifier_clone.send("文本剪贴板已同步", &preview);
                    }
                }
                network::ClipboardContent::Image {
                    width,
                    height,
                    data,
                } => {
                    if let Err(e) = clipboard_clone.set_image(*width, *height, data) {
                        eprintln!("更新图片剪贴板失败: {}", e);
                    } else {
                        let preview = format!("图片 {}x{}", width, height);
                        let _ = notifier_clone.send("图片剪贴板已同步", &preview);
                    }
                }
            }
        }
    });

    println!("🌐 正在自动搜索局域网内的其他设备...");
    println!("📋 监控剪贴板变化中...");
    println!("按 Ctrl+C 停止服务");

    // 剪贴板监控循环
    let mut last_text_content = String::new();
    let mut last_content_type = clipboard::ClipboardContentType::Empty;

    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 检查剪贴板内容类型
        let current_type = clipboard.get_content_type();

        match current_type {
            clipboard::ClipboardContentType::Text => {
                if let Ok(current_content) = clipboard.get_text() {
                    if current_content != last_text_content && !current_content.is_empty() {
                        println!("检测到文本剪贴板变化: {}", current_content);

                        // 广播文本到其他设备
                        if let Err(e) = network.broadcast_clipboard(&current_content).await {
                            eprintln!("文本广播失败: {}", e);
                        }

                        last_text_content = current_content;
                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Image => {
                // 只有当之前不是图片类型时才处理，避免重复处理
                if !matches!(last_content_type, clipboard::ClipboardContentType::Image) {
                    if let Ok(Some((width, height, png_data))) = clipboard.get_image() {
                        println!("检测到图片剪贴板变化: {}x{}", width, height);

                        // 广播图片到其他设备
                        if let Err(e) = network.broadcast_image(width, height, png_data).await {
                            eprintln!("图片广播失败: {}", e);
                        }

                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Empty => {
                // 剪贴板为空，更新状态
                if !matches!(last_content_type, clipboard::ClipboardContentType::Empty) {
                    last_content_type = current_type;
                    last_text_content.clear();
                }
            }
        }

        // 检查退出信号
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }

    network.shutdown().await;
    println!("自动连接服务已停止");

    Ok(())
}

/// 测试剪贴板功能
async fn test_clipboard(manager: ClipboardManager) -> Result<()> {
    println!("测试剪贴板功能...");

    // 读取当前剪贴板内容
    if let Ok(current) = manager.get_text() {
        println!("当前剪贴板内容: {}", current);
    }

    // 写入测试内容
    let test_text = "这是来自 Rust 程序的测试内容！";
    manager.set_text(test_text)?;
    println!("已写入测试内容到剪贴板");

    // 验证写入成功
    let result = manager.get_text()?;
    println!("读取到的内容: {}", result);

    println!("剪贴板测试完成！");
    Ok(())
}

/// 运行同步服务
async fn run_sync_service(clipboard: ClipboardManager, network: NetworkManager) -> Result<()> {
    let notifier = NotificationManager::new();

    println!("启动剪贴板同步服务...");

    // 发送启动通知
    notifier.send("剪贴板同步", "同步服务已启动")?;

    // 显示连接信息
    let ticket = network.generate_ticket().await?;
    println!("节点 ID: {}", network.get_node_id());
    println!("连接票据: {}", ticket);
    println!("\n其他设备可以使用以下命令连接到此设备:");
    println!("clipboard-sync -- connect {}", ticket);
    println!("");
    println!("正在监听连接和剪贴板变化...");
    println!("按 Ctrl+C 停止服务");

    // 设置消息处理器
    let mut message_receiver = network.setup_message_handler().await;

    // 启动网络监听任务
    // let network_clone = network.clone();
    // tokio::spawn(async move {
    //     if let Err(e) = network_clone.listen_for_connections().await {
    //         eprintln!("网络监听失败: {}", e);
    //     }
    // });

    // 启动消息处理任务
    let clipboard_clone = clipboard.clone();
    let notifier_clone = notifier.clone();
    tokio::spawn(async move {
        while let Some(message) = message_receiver.recv().await {
            println!(
                "收到剪贴板消息: {} (来自: {})",
                message.content, message.sender_id
            );

            // 根据消息类型更新本地剪贴板
            match &message.content {
                network::ClipboardContent::Text(text) => {
                    if let Err(e) = clipboard_clone.set_text(text) {
                        eprintln!("更新文本剪贴板失败: {}", e);
                    } else {
                        let preview = message.content.preview(50);
                        let _ = notifier_clone.send("文本剪贴板已同步", &preview);
                    }
                }
                network::ClipboardContent::Image {
                    width,
                    height,
                    data,
                } => {
                    if let Err(e) = clipboard_clone.set_image(*width, *height, data) {
                        eprintln!("更新图片剪贴板失败: {}", e);
                    } else {
                        let preview = format!("图片 {}x{}", width, height);
                        let _ = notifier_clone.send("图片剪贴板已同步", &preview);
                    }
                }
            }
        }
    });

    // 剪贴板监控循环
    let mut last_text_content = String::new();
    let mut last_content_type = clipboard::ClipboardContentType::Empty;

    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 检查剪贴板内容类型
        let current_type = clipboard.get_content_type();

        match current_type {
            clipboard::ClipboardContentType::Text => {
                if let Ok(current_content) = clipboard.get_text() {
                    if current_content != last_text_content && !current_content.is_empty() {
                        println!("检测到文本剪贴板变化: {}", current_content);

                        // 广播文本到其他设备
                        if let Err(e) = network.broadcast_clipboard(&current_content).await {
                            eprintln!("文本广播失败: {}", e);
                        }

                        last_text_content = current_content;
                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Image => {
                // 只有当之前不是图片类型时才处理，避免重复处理
                if !matches!(last_content_type, clipboard::ClipboardContentType::Image) {
                    if let Ok(Some((width, height, png_data))) = clipboard.get_image() {
                        println!("检测到图片剪贴板变化: {}x{}", width, height);

                        // 广播图片到其他设备
                        if let Err(e) = network.broadcast_image(width, height, png_data).await {
                            eprintln!("图片广播失败: {}", e);
                        }

                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Empty => {
                // 剪贴板为空，更新状态
                if !matches!(last_content_type, clipboard::ClipboardContentType::Empty) {
                    last_content_type = current_type;
                    last_text_content.clear();
                }
            }
        }

        // 检查退出信号
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }

    network.shutdown().await;
    println!("同步服务已停止");

    Ok(())
}

/// 连接到其他设备
async fn connect_to_peer(
    clipboard: ClipboardManager,
    network: NetworkManager,
    ticket: &str,
) -> Result<()> {
    let notifier = NotificationManager::new();

    println!("正在连接到其他设备...");

    network.connect_to_peer(ticket).await?;

    println!("连接成功！开始同步剪贴板内容...");
    notifier.send("剪贴板同步", "已连接到其他设备")?;

    println!("按 Ctrl+C 断开连接");

    // 设置消息处理器
    let mut message_receiver = network.setup_message_handler().await;

    // 启动网络监听任务
    // let network_clone = network.clone();
    // tokio::spawn(async move {
    //     if let Err(e) = network_clone.listen_for_connections().await {
    //         eprintln!("网络监听失败: {}", e);
    //     }
    // });

    // 启动消息处理任务
    let clipboard_clone = clipboard.clone();
    let notifier_clone = notifier.clone();
    tokio::spawn(async move {
        while let Some(message) = message_receiver.recv().await {
            println!(
                "收到剪贴板消息: {} (来自: {})",
                message.content, message.sender_id
            );

            // 根据消息类型更新本地剪贴板
            match &message.content {
                network::ClipboardContent::Text(text) => {
                    if let Err(e) = clipboard_clone.set_text(text) {
                        eprintln!("更新文本剪贴板失败: {}", e);
                    } else {
                        let preview = message.content.preview(50);
                        let _ = notifier_clone.send("文本剪贴板已同步", &preview);
                    }
                }
                network::ClipboardContent::Image {
                    width,
                    height,
                    data,
                } => {
                    if let Err(e) = clipboard_clone.set_image(*width, *height, data) {
                        eprintln!("更新图片剪贴板失败: {}", e);
                    } else {
                        let preview = format!("图片 {}x{}", width, height);
                        let _ = notifier_clone.send("图片剪贴板已同步", &preview);
                    }
                }
            }
        }
    });

    // 剪贴板监控循环
    let mut last_text_content = String::new();
    let mut last_content_type = clipboard::ClipboardContentType::Empty;

    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 检查剪贴板内容类型
        let current_type = clipboard.get_content_type();

        match current_type {
            clipboard::ClipboardContentType::Text => {
                if let Ok(current_content) = clipboard.get_text() {
                    if current_content != last_text_content && !current_content.is_empty() {
                        println!("检测到文本剪贴板变化: {}", current_content);

                        // 广播文本到其他设备
                        if let Err(e) = network.broadcast_clipboard(&current_content).await {
                            eprintln!("文本广播失败: {}", e);
                        }

                        last_text_content = current_content;
                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Image => {
                // 只有当之前不是图片类型时才处理，避免重复处理
                if !matches!(last_content_type, clipboard::ClipboardContentType::Image) {
                    if let Ok(Some((width, height, png_data))) = clipboard.get_image() {
                        println!("检测到图片剪贴板变化: {}x{}", width, height);

                        // 广播图片到其他设备
                        if let Err(e) = network.broadcast_image(width, height, png_data).await {
                            eprintln!("图片广播失败: {}", e);
                        }

                        last_content_type = current_type;
                    }
                }
            }
            clipboard::ClipboardContentType::Empty => {
                // 剪贴板为空，更新状态
                if !matches!(last_content_type, clipboard::ClipboardContentType::Empty) {
                    last_content_type = current_type;
                    last_text_content.clear();
                }
            }
        }

        // 检查退出信号
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }

    network.shutdown().await;
    println!("连接已断开");

    Ok(())
}

use anyhow::Result;
use iroh::protocol::{ProtocolHandler, Router, AcceptError};
use iroh::{Endpoint, NodeAddr, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::future::Future;
use tokio::sync::{mpsc, Mutex};
use tokio::io::AsyncReadExt;
use futures_lite::StreamExt;

// 定义我们的协议ALPN
const CLIPBOARD_ALPN: &[u8] = b"iroh-clipboard-sync/0";

/// 剪贴板协议处理器
#[derive(Debug, Clone)]
pub struct ClipboardProtocol {
    message_sender: Arc<Mutex<Option<mpsc::UnboundedSender<ClipboardMessage>>>>,
}

impl ClipboardProtocol {
    pub fn new() -> Self {
        Self {
            message_sender: Arc::new(Mutex::new(None)),
        }
    }
    
    pub async fn set_message_sender(&self, sender: mpsc::UnboundedSender<ClipboardMessage>) {
        *self.message_sender.lock().await = Some(sender);
    }
}

impl ProtocolHandler for ClipboardProtocol {
    fn accept(&self, connection: iroh::endpoint::Connection) -> impl Future<Output = Result<(), AcceptError>> + Send {
        let message_sender = self.message_sender.clone();
        
        async move {
            println!("接受剪贴板协议连接");
            
            // 接受双向流
            let result = connection.accept_bi().await;
            let (_send_stream, mut recv_stream) = match result {
                Ok(streams) => streams,
                Err(e) => {
                    eprintln!("Failed to accept bidirectional stream: {}", e);
                    return Err(AcceptError::from(e));
                }
            };
            
            // 读取消息
            let mut buf = [0u8; 4096];
            while let Ok(Some(n)) = recv_stream.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                
                match ClipboardMessage::from_bytes(&buf[..n]) {
                    Ok(message) => {
                        match &message.content {
                            ClipboardContent::Text(text) => {
                                println!("收到文本消息: {} (来自: {})", text, message.sender_id);
                            }
                            ClipboardContent::Image { width, height, .. } => {
                                println!("收到图片消息: {}x{} (来自: {})", width, height, message.sender_id);
                            }
                        }
                        
                        if let Some(sender) = message_sender.lock().await.as_ref() {
                            let _ = sender.send(message);
                        }
                    }
                    Err(e) => {
                        eprintln!("消息解析失败: {}", e);
                    }
                }
            }
            
            Ok(())
        }
    }
}

/// 剪贴板内容类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClipboardContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        data: Vec<u8>, // PNG 格式的图片数据
    },
}

impl std::fmt::Display for ClipboardContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardContent::Text(text) => write!(f, "文本: {}", text),
            ClipboardContent::Image { width, height, .. } => {
                write!(f, "图片: {}x{}", width, height)
            }
        }
    }
}

impl ClipboardContent {
    /// 获取内容长度（用于预览）
    pub fn preview_length(&self) -> usize {
        match self {
            ClipboardContent::Text(text) => text.len(),
            ClipboardContent::Image { .. } => 50, // 图片固定长度
        }
    }
    
    /// 获取内容预览字符串
    pub fn preview(&self, max_length: usize) -> String {
        match self {
            ClipboardContent::Text(text) => {
                // 使用字符迭代器来安全地截取UTF-8字符串
                let char_count = text.chars().count();
                if char_count > max_length {
                    let truncated: String = text.chars().take(max_length).collect();
                    format!("{}...", truncated)
                } else {
                    text.clone()
                }
            }
            ClipboardContent::Image { width, height, .. } => {
                format!("图片 {}x{}", width, height)
            }
        }
    }
}

/// 剪贴板同步消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardMessage {
    pub content: ClipboardContent,
    pub timestamp: u64, // Unix 时间戳
    pub sender_id: String, // 发送者标识
}

impl ClipboardMessage {
    /// 创建文本消息
    pub fn new_text(content: String, sender_id: String) -> Self {
        Self {
            content: ClipboardContent::Text(content),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            sender_id,
        }
    }
    
    /// 创建图片消息
    pub fn new_image(width: u32, height: u32, data: Vec<u8>, sender_id: String) -> Self {
        Self {
            content: ClipboardContent::Image { width, height, data },
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            sender_id,
        }
    }

    /// 序列化为字节
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(Into::into)
    }

    /// 从字节反序列化
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }
}

/// 网络连接票据 - 用于设备间连接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTicket {
    pub node_id: NodeId,
    pub addresses: Vec<SocketAddr>,
}

impl ConnectionTicket {
    /// 序列化为字符串
    pub fn to_string(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(json))
    }

    /// 从字符串反序列化
    pub fn from_string(s: &str) -> Result<Self> {
        use base64::Engine;
        let json = base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| anyhow::anyhow!("Base64解码失败: {}", e))?;
        let json_str = String::from_utf8(json)?;
        serde_json::from_str(&json_str).map_err(Into::into)
    }
}

/// P2P 网络管理器
#[derive(Clone)]
pub struct NetworkManager {
    router: Router,
    device_name: String,
    protocol: ClipboardProtocol,
    connections: Arc<Mutex<HashMap<NodeId, iroh::endpoint::Connection>>>,
}

impl NetworkManager {
    /// 创建新的网络管理器
    pub async fn new(device_name: String) -> Result<Self> {
        println!("正在启动 P2P 网络...");
        
        // 创建 endpoint，启用本地网络发现
        let endpoint = Endpoint::builder()
            .discovery_local_network() // 这是关键！启用局域网设备发现
            .bind()
            .await
            .map_err(|e| anyhow::anyhow!("网络初始化失败: {}", e))?;

        println!("网络节点 ID: {}", endpoint.node_id());
        
        // 创建协议处理器
        let protocol = ClipboardProtocol::new();
        
        // 创建 Router
        let router = Router::builder(endpoint)
            .accept(CLIPBOARD_ALPN, protocol.clone())
            .spawn();
        
        Ok(Self {
            router,
            device_name,
            protocol,
            connections: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 获取当前节点信息
    pub fn get_node_id(&self) -> NodeId {
        self.router.endpoint().node_id()
    }

    /// 生成连接票据
    pub async fn generate_ticket(&self) -> Result<String> {
        // 使用简化的方法，只提供节点ID
        let node_id = self.router.endpoint().node_id();
        let ticket = ConnectionTicket {
            node_id,
            addresses: vec![], // 暂时为空，依赖iroh的发现机制
        };
        ticket.to_string()
    }

    /// 连接到其他设备
    pub async fn connect_to_peer(&self, ticket_str: &str) -> Result<()> {
        let ticket = ConnectionTicket::from_string(ticket_str)?;
        
        // 构建节点地址
        let node_addr = NodeAddr::new(ticket.node_id).with_direct_addresses(ticket.addresses);
        
        println!("正在连接到设备: {}", ticket.node_id);
        
        // 连接到目标节点，使用正确的ALPN
        let connection = self.router.endpoint().connect(node_addr, CLIPBOARD_ALPN).await?;
        
        println!("成功连接到设备！");
        
        // 保存连接
        self.connections.lock().await.insert(ticket.node_id, connection);
        
        Ok(())
    }

    /// 初始化消息处理器
    pub async fn setup_message_handler(&self) -> mpsc::UnboundedReceiver<ClipboardMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.protocol.set_message_sender(tx).await;
        rx
    }

    /// 监听传入的连接 - Router会自动处理
    pub async fn listen_for_connections(&self) -> Result<()> {
        // Router已经在后台自动处理连接，这里只是保持连接活跃
        println!("网络路由器已启动，正在监听连接...");
        
        // 保持连接状态
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    }
    

    /// 发送剪贴板消息到所有连接的设备
    pub async fn broadcast_message(&self, message: ClipboardMessage) -> Result<()> {
        let data = message.to_bytes()?;
        
        // 记录日志
        match &message.content {
            ClipboardContent::Text(text) => {
                println!("广播文本内容: {}", text);
            }
            ClipboardContent::Image { width, height, .. } => {
                println!("广播图片内容: {}x{}", width, height);
            }
        }
        
        // 向所有连接的设备发送消息
        let connections = self.connections.lock().await;
        let mut failed_connections = Vec::new();
        
        for (node_id, connection) in connections.iter() {
            // 为每个连接打开一个新的双向流
            match connection.open_bi().await {
                Ok((mut send_stream, _recv_stream)) => {
                    match send_stream.write_all(&data).await {
                        Ok(_) => {
                            println!("消息已发送到: {}", node_id);
                            let _ = send_stream.finish();
                        }
                        Err(e) => {
                            eprintln!("发送到 {} 失败: {}", node_id, e);
                            failed_connections.push(*node_id);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("打开流到 {} 失败: {}", node_id, e);
                    failed_connections.push(*node_id);
                }
            }
        }
        
        // 清理失败的连接
        drop(connections);
        if !failed_connections.is_empty() {
            let mut connections = self.connections.lock().await;
            for node_id in failed_connections {
                connections.remove(&node_id);
            }
        }
        
        Ok(())
    }

    /// 广播文本内容到所有连接的设备
    pub async fn broadcast_clipboard(&self, content: &str) -> Result<()> {
        let message = ClipboardMessage::new_text(
            content.to_string(), 
            self.device_name.clone()
        );
        self.broadcast_message(message).await
    }
    
    /// 广播图片内容到所有连接的设备
    pub async fn broadcast_image(&self, width: u32, height: u32, data: Vec<u8>) -> Result<()> {
        let message = ClipboardMessage::new_image(
            width, 
            height, 
            data, 
            self.device_name.clone()
        );
        self.broadcast_message(message).await
    }

    /// 尝试连接到一个可能的其他剪贴板节点
    pub async fn try_connect_to_clipboard_node(&self, node_id: NodeId) -> Result<()> {
        // 检查是否已经连接
        if self.connections.lock().await.contains_key(&node_id) {
            return Ok(()); // 已经连接了
        }
        
        // 不要连接自己
        if node_id == self.get_node_id() {
            return Ok(());
        }
        
        println!("尝试连接到发现的节点: {}", node_id);
        
        // 构建节点地址（只有NodeId，依赖iroh的发现机制找到地址）
        let node_addr = NodeAddr::new(node_id);
        
        // 尝试连接
        match self.router.endpoint().connect(node_addr, CLIPBOARD_ALPN).await {
            Ok(connection) => {
                println!("✅ 成功连接到节点: {}", node_id);
                
                // 保存连接
                self.connections.lock().await.insert(node_id, connection);
                Ok(())
            }
            Err(e) => {
                // 连接失败是正常的，可能这个节点不是剪贴板同步程序
                println!("❌ 连接到节点 {} 失败: {} (可能不是剪贴板同步程序)", node_id, e);
                Err(e.into())
            }
        }
    }
    
    /// 自动发现并连接局域网内的其他剪贴板同步节点
    pub async fn start_auto_discovery(&self) -> Result<()> {
        println!("🔍 启动自动发现服务...");
        
        let my_node_id = self.get_node_id();
        println!("💻 本机节点 ID: {}", my_node_id);
        
        // 获取发现事件流
        let mut discovery_stream = self.router.endpoint().discovery_stream();
        let connections = self.connections.clone();
        
        println!("🌐 正在扫描局域网内的其他设备...");
        
        // 处理发现事件
        while let Some(event_result) = discovery_stream.next().await {
            match event_result {
                Ok(discovered_node) => {
                    let discovered_node_id = discovered_node.node_id();
                    
                    // 不要连接自己
                    if discovered_node_id == my_node_id {
                        continue;
                    }
                    
                    // 检查是否已经连接
                    if connections.lock().await.contains_key(&discovered_node_id) {
                        continue;
                    }
                    
                    println!("🎆 发现新设备: {}", discovered_node_id);
                    
                    // 尝试连接
                    if let Err(_) = self.try_connect_to_clipboard_node(discovered_node_id).await {
                        // 连接失败是正常的，可能不是剪贴板同步程序
                    }
                },
                Err(e) => {
                    eprintln!("发现服务错误: {}", e);
                }
            }
        }
        
        println!("🚫 发现流结束");
        Ok(())
    }

    /// 关闭网络管理器
    pub async fn shutdown(self) {
        println!("正在关闭网络连接...");
        if let Err(e) = self.router.shutdown().await {
            eprintln!("关闭路由器失败: {}", e);
        }
        println!("网络已关闭");
    }
}

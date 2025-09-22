use anyhow::Result;
use arboard::{Clipboard, ImageData};
use std::sync::{Arc, Mutex};
use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

/// 剪贴板内容类型
#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardContentType {
    Text,
    Image,
    Empty,
}

/// 剪贴板管理器 - 负责读写剪贴板内容
#[derive(Clone)]
pub struct ClipboardManager {
    clipboard: Arc<Mutex<Clipboard>>,
}

impl ClipboardManager {
    /// 创建新的剪贴板管理器
    pub fn new() -> Result<Self> {
        let clipboard = Clipboard::new()
            .map_err(|e| anyhow::anyhow!("无法初始化剪贴板: {}", e))?;
        
        Ok(Self {
            clipboard: Arc::new(Mutex::new(clipboard)),
        })
    }

    /// 获取剪贴板中的文字内容
    pub fn get_text(&self) -> Result<String> {
        let mut clipboard = self.clipboard.lock().unwrap();
        clipboard.get_text()
            .map_err(|e| anyhow::anyhow!("读取剪贴板失败: {}", e))
    }

    /// 设置剪贴板文字内容
    pub fn set_text(&self, text: &str) -> Result<()> {
        let mut clipboard = self.clipboard.lock().unwrap();
        clipboard.set_text(text)
            .map_err(|e| anyhow::anyhow!("写入剪贴板失败: {}", e))
    }

    /// 获取剪贴板中的图片内容
    pub fn get_image(&self) -> Result<Option<(u32, u32, Vec<u8>)>> {
        let mut clipboard = self.clipboard.lock().unwrap();
        match clipboard.get_image() {
            Ok(image_data) => {
                // 将 RGBA 数据转换为 PNG 格式
                let png_data = self.rgba_to_png(&image_data)?;
                Ok(Some((image_data.width as u32, image_data.height as u32, png_data)))
            }
            Err(_) => Ok(None),
        }
    }
    
    /// 设置剪贴板图片内容
    pub fn set_image(&self, width: u32, height: u32, png_data: &[u8]) -> Result<()> {
        let mut clipboard = self.clipboard.lock().unwrap();
        
        // 将 PNG 数据转换为 RGBA
        let image_data = self.png_to_rgba(width, height, png_data)?;
        clipboard.set_image(image_data)
            .map_err(|e| anyhow::anyhow!("写入剪贴板图片失败: {}", e))
    }
    
    /// 检测剪贴板内容类型
    pub fn get_content_type(&self) -> ClipboardContentType {
        let mut clipboard = self.clipboard.lock().unwrap();
        
        // 先检查是否有图片
        if clipboard.get_image().is_ok() {
            return ClipboardContentType::Image;
        }
        
        // 再检查是否有文本
        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                return ClipboardContentType::Text;
            }
        }
        
        ClipboardContentType::Empty
    }
    
    /// 检查剪贴板是否有内容
    pub fn has_content(&self) -> bool {
        !matches!(self.get_content_type(), ClipboardContentType::Empty)
    }
    
    /// 将 RGBA 数据转换为 PNG 格式
    fn rgba_to_png(&self, image_data: &ImageData) -> Result<Vec<u8>> {
        let rgba_image = RgbaImage::from_raw(
            image_data.width as u32, 
            image_data.height as u32, 
            image_data.bytes.to_vec()
        ).ok_or_else(|| anyhow::anyhow!("无法创建 RGBA 图像"))?;
        
        let mut png_data = Vec::new();
        let mut cursor = Cursor::new(&mut png_data);
        
        rgba_image.write_to(&mut cursor, ImageFormat::Png)
            .map_err(|e| anyhow::anyhow!("PNG 编码失败: {}", e))?;
        
        Ok(png_data)
    }
    
    /// 将 PNG 数据转换为 RGBA 格式
    fn png_to_rgba(&self, width: u32, height: u32, png_data: &[u8]) -> Result<ImageData> {
        let cursor = Cursor::new(png_data);
        let img = image::load(cursor, ImageFormat::Png)
            .map_err(|e| anyhow::anyhow!("PNG 解码失败: {}", e))?;
        
        let rgba_img = img.to_rgba8();
        let bytes = rgba_img.into_raw();
        
        Ok(ImageData {
            width: width as usize,
            height: height as usize,
            bytes: bytes.into(),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_basic_operations() {
        let manager = ClipboardManager::new().expect("创建剪贴板管理器失败");
        
        // 测试写入和读取
        let test_text = "Hello, Clipboard!";
        manager.set_text(test_text).expect("写入失败");
        
        let result = manager.get_text().expect("读取失败");
        assert_eq!(result, test_text);
    }
}
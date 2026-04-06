//! pdfxml - 将 XFDF/XML 格式的 PDF 注释导出为 PDF 文件
//!
//! XFDF (XML Forms Data Format) 是 Adobe 定义的 XML 格式，
//! 用于表示 PDF 文档中的表单数据和注释。

mod xfdf;
mod pdf;
mod error;
mod annotation;

use anyhow::Result;
use clap::Parser;
use log::{info, warn};
use std::fs;
use std::path::PathBuf;

use crate::xfdf::XfdfDocument;
use crate::pdf::PdfAnnotationExporter;

/// 命令行参数
#[derive(Parser, Debug)]
#[command(name = "pdfxml")]
#[command(about = "将 XFDF/XML 注释导出为 PDF 文件", long_about = None)]
#[command(version = "0.1.0")]
struct Args {
    /// 输入的 XFDF/XML 文件路径
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// 输出的 PDF 文件路径
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// 目标 PDF 文件路径（可选，如果提供则将注释合并到现有PDF）
    #[arg(short, long, value_name = "FILE")]
    target_pdf: Option<PathBuf>,

    /// 详细输出模式
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 初始化日志
    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    info!("开始处理 XFDF 文件: {:?}", args.input);

    // 验证输入文件存在
    if !args.input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", args.input));
    }

    // 确定输出路径
    let output_path = match &args.output {
        Some(path) => path.clone(),
        None => {
            // 默认输出路径：与输入同名但扩展名为 .pdf
            let mut default_output = args.input.clone();
            default_output.set_extension("pdf");
            default_output
        }
    };

    // 读取并解析 XFDF 文件
    let xml_content = fs::read_to_string(&args.input)?;
    info!("成功读取 XML 文件，大小: {} 字节", xml_content.len());

    let xfdf_doc = XfdfDocument::parse(&xml_content)?;
    info!("解析完成，发现 {} 条注释", xfdf_doc.annotations.len());

    if xfdf_doc.annotations.is_empty() {
        warn!("警告：未找到任何注释");
    }

    // 创建 PDF 导出器并导出
    let mut exporter = PdfAnnotationExporter::new();

    match &args.target_pdf {
        Some(target_path) => {
            // 合并到现有 PDF
            if !target_path.exists() {
                return Err(anyhow::anyhow!("目标 PDF 文件不存在: {:?}", target_path));
            }
            info!("将注释合并到现有 PDF: {:?}", target_path);
            exporter.export_to_existing_pdf(&xfdf_doc, target_path, &output_path)?;
        }
        None => {
            // 创建新 PDF（仅包含注释信息）
            info!("创建新的 PDF 文件: {:?}", output_path);
            exporter.export_to_new_pdf(&xfdf_doc, &output_path)?;
        }
    }

    info!("导出成功！输出文件: {:?}", output_path);
    println!("✓ 注释已成功导出到: {}", output_path.display());
    
    // 输出摘要
    println!("\n📊 导出摘要:");
    println!("  - 总计注释数: {}", xfdf_doc.annotations.len());
    
    // 按类型统计
    let mut type_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for annot in &xfdf_doc.annotations {
        *type_counts.entry(annot.annotation_type()).or_insert(0) += 1;
    }
    for (typ, count) in type_counts.iter() {
        println!("  - {}: {} 条", typ, count);
    }

    Ok(())
}

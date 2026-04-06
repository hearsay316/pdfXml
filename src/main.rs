//! pdfxml - 将 XFDF/XML 格式的 PDF 注释导出为 PDF 文件
//!
//! XFDF (XML Forms Data Format) 是 Adobe 定义的 XML 格式，
//! 用于表示 PDF 文档中的表单数据和注释。

use anyhow::Result;
use clap::Parser;
use log::{info, warn};
use pdfxml::{load_xfdf, export_annotations};
use std::path::PathBuf;

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

    if !args.input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", args.input));
    }

    let output_path = match &args.output {
        Some(path) => path.clone(),
        None => {
            let mut default_output = args.input.clone();
            default_output.set_extension("pdf");
            default_output
        }
    };

    let xfdf_doc = load_xfdf(&args.input)?;
    info!("解析完成，发现 {} 条注释", xfdf_doc.annotations.len());

    if xfdf_doc.annotations.is_empty() {
        warn!("警告：未找到任何注释");
    }

    if let Some(target_path) = &args.target_pdf {
        if !target_path.exists() {
            return Err(anyhow::anyhow!("目标 PDF 文件不存在: {:?}", target_path));
        }
        info!("将注释合并到现有 PDF: {:?}", target_path);
    } else {
        info!("创建新的 PDF 文件: {:?}", output_path);
    }

    export_annotations(&xfdf_doc, args.target_pdf.as_ref(), &output_path)?;

    info!("导出成功！输出文件: {:?}", output_path);
    println!("✓ 注释已成功导出到: {}", output_path.display());
    println!("\n📊 导出摘要:");
    println!("  - 总计注释数: {}", xfdf_doc.annotations.len());

    let mut type_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for annot in &xfdf_doc.annotations {
        *type_counts.entry(annot.annotation_type()).or_insert(0) += 1;
    }
    for (typ, count) in type_counts.iter() {
        println!("  - {}: {} 条", typ, count);
    }

    Ok(())
}

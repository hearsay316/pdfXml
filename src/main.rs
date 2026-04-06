//! pdfxml 命令行工具。
//!
//! 这个文件故意保持很薄：
//! 它只负责“接收命令行参数 → 调用库 → 打印结果”。
//! 真正的解析和导出逻辑都放在 `src/lib.rs` 和其它模块里，
//! 这样以后既能继续做 CLI，也能把同一套能力当成 SDK 给别的 Rust 项目调用。

use anyhow::Result;
use clap::Parser;
use log::{info, warn};
use pdfxml::{
    export_annotations,
    export_pdf_annotations_to_xfdf,
    load_annotations_from_pdf,
    load_xfdf,
};
use std::path::PathBuf;

/// 命令行参数。
///
/// 可以把它理解成“用户在终端里输入的选项说明书”。
#[derive(Parser, Debug)]
#[command(name = "pdfxml")]
#[command(about = "XFDF 与 PDF 注释互转工具", long_about = None)]
#[command(version = "0.1.0")]
struct Args {
    /// 输入文件路径。
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// 输出文件路径。
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// 目标 PDF 文件路径。
    ///
    /// 仅在“XFDF -> PDF”模式下生效。
    #[arg(short, long, value_name = "FILE")]
    target_pdf: Option<PathBuf>,

    /// 从 PDF 导出 XFDF。
    #[arg(long)]
    from_pdf: bool,

    /// 是否输出更详细的日志。
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

    if !args.input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", args.input));
    }

    if args.from_pdf {
        info!("开始从 PDF 导出 XFDF: {:?}", args.input);

        let output_path = match &args.output {
            Some(path) => path.clone(),
            None => {
                let mut default_output = args.input.clone();
                default_output.set_extension("xfdf");
                default_output
            }
        };

        let xfdf_doc = load_annotations_from_pdf(&args.input)?;
        info!("读取完成，发现 {} 条注释", xfdf_doc.annotations.len());

        if xfdf_doc.annotations.is_empty() {
            warn!("警告：未找到任何注释");
        }

        export_pdf_annotations_to_xfdf(&args.input, &output_path)?;

        info!("导出成功！输出文件: {:?}", output_path);
        println!("✓ XFDF 已成功导出到: {}", output_path.display());
        println!("\n导出摘要:");
        println!("  - 总计注释数: {}", xfdf_doc.annotations.len());
        return Ok(());
    }

    info!("开始处理 XFDF 文件: {:?}", args.input);

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
    println!("\n导出摘要:");
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

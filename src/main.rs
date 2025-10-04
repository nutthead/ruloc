//! # ruloc - Rust Lines of Code Counter
//!
//! A minimalist tool for counting lines of code in Rust source files.
//! Provides detailed statistics including total, production, and test code metrics.

use clap::{Parser, ValueEnum};
use log::{debug, trace};
use ra_ap_syntax::{ast, ast::HasAttrs, AstNode, SourceFile, SyntaxNode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ============================================================================
// Data Structures
// ============================================================================

/// Statistics for lines of code in a given scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineStats {
    /// Total number of lines including everything.
    #[serde(rename = "all-lines")]
    pub all_lines: usize,
    /// Number of blank lines (whitespace only).
    #[serde(rename = "blank-lines")]
    pub blank_lines: usize,
    /// Number of comment lines.
    #[serde(rename = "comment-lines")]
    pub comment_lines: usize,
    /// Number of actual code lines.
    #[serde(rename = "code-lines")]
    pub code_lines: usize,
}

impl LineStats {
    /// Adds another `LineStats` to this one.
    pub fn add(&mut self, other: &LineStats) {
        self.all_lines += other.all_lines;
        self.blank_lines += other.blank_lines;
        self.comment_lines += other.comment_lines;
        self.code_lines += other.code_lines;
    }
}

/// Statistics for a single file, broken down by total, production, and test code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStats {
    /// Path to the file relative to the analysis root.
    pub path: String,
    /// Statistics for all code in the file.
    pub total: LineStats,
    /// Statistics for production code only.
    pub production: LineStats,
    /// Statistics for test code only.
    pub test: LineStats,
}

/// Summary statistics across all analyzed files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    /// Total number of files analyzed.
    pub files: usize,
    /// Aggregate statistics for all code.
    pub total: LineStats,
    /// Aggregate statistics for production code.
    pub production: LineStats,
    /// Aggregate statistics for test code.
    pub test: LineStats,
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            files: 0,
            total: LineStats::default(),
            production: LineStats::default(),
            test: LineStats::default(),
        }
    }
}

impl Summary {
    /// Adds file statistics to this summary.
    pub fn add_file(&mut self, file_stats: &FileStats) {
        self.files += 1;
        self.total.add(&file_stats.total);
        self.production.add(&file_stats.production);
        self.test.add(&file_stats.test);
    }
}

/// Complete analysis report including summary and per-file statistics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    /// Summary of all analyzed files.
    pub summary: Summary,
    /// Individual statistics for each file.
    pub files: Vec<FileStats>,
}

// ============================================================================
// CLI
// ============================================================================

/// Output format for the report.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    /// Plain text format (default).
    Text,
    /// JSON format.
    Json,
}

/// Command-line arguments for ruloc.
#[derive(Debug, Parser)]
#[command(name = "ruloc", version, about = "Rust lines of code counter")]
struct Args {
    /// Analyze a single Rust file.
    #[arg(short, long, value_name = "FILE", conflicts_with = "dir")]
    file: Option<PathBuf>,

    /// Analyze all Rust files in a directory recursively.
    #[arg(short, long, value_name = "DIR", conflicts_with = "file")]
    dir: Option<PathBuf>,

    /// Output in plain text format (default).
    #[arg(long, conflicts_with = "out_json")]
    out_text: bool,

    /// Output in JSON format.
    #[arg(long, conflicts_with = "out_text")]
    out_json: bool,

    /// Enable verbose output for debugging.
    #[arg(long)]
    verbose: bool,
}

impl Args {
    /// Determines the output format based on flags.
    fn output_format(&self) -> OutputFormat {
        if self.out_json {
            OutputFormat::Json
        } else {
            OutputFormat::Text
        }
    }
}

// ============================================================================
// Line Analysis
// ============================================================================

/// Classifies line types in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineType {
    /// Line contains only whitespace.
    Blank,
    /// Line is part of a comment.
    Comment,
    /// Line contains code.
    Code,
}

/// Analyzes the content of source code to classify each line.
fn analyze_lines(content: &str) -> Vec<LineType> {
    let mut line_types = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            line_types.push(LineType::Blank);
            continue;
        }

        // Check for block comment start/end
        if in_block_comment {
            line_types.push(LineType::Comment);
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }

        if trimmed.starts_with("/*") {
            line_types.push(LineType::Comment);
            if !trimmed.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }

        // Line comments or doc comments
        if trimmed.starts_with("//") {
            line_types.push(LineType::Comment);
            continue;
        }

        // Otherwise, it's code
        line_types.push(LineType::Code);
    }

    line_types
}

/// Computes line statistics from classified line types.
fn compute_line_stats(line_types: &[LineType], total_lines: usize) -> LineStats {
    let blank_lines = line_types.iter().filter(|&&t| t == LineType::Blank).count();
    let comment_lines = line_types.iter().filter(|&&t| t == LineType::Comment).count();
    let code_lines = line_types.iter().filter(|&&t| t == LineType::Code).count();

    LineStats {
        all_lines: total_lines,
        blank_lines,
        comment_lines,
        code_lines,
    }
}

// ============================================================================
// Production vs Test Detection
// ============================================================================

/// Represents a code section with its classification and line range.
#[derive(Debug, Clone)]
struct CodeSection {
    /// Starting line number (0-indexed).
    start_line: usize,
    /// Ending line number (0-indexed, inclusive).
    end_line: usize,
}

/// Determines if a syntax node represents a test item.
fn is_test_node(node: &SyntaxNode) -> bool {
    // Check if this is a function with #[test] or #[cfg(test)] attribute
    if let Some(func) = ast::Fn::cast(node.clone()) {
        for attr in func.attrs() {
            if let Some(path) = attr.path() {
                let attr_text = path.to_string();
                if attr_text == "test" || attr_text.contains("cfg(test)") {
                    return true;
                }
            }
        }
    }

    // Check if this is a module with #[cfg(test)]
    if let Some(module) = ast::Module::cast(node.clone()) {
        for attr in module.attrs() {
            if let Some(path) = attr.path() {
                let attr_text = path.to_string();
                if attr_text.contains("cfg(test)") {
                    return true;
                }
            }
        }
    }

    false
}

/// Recursively finds test sections in the syntax tree.
fn find_test_sections(node: &SyntaxNode, sections: &mut Vec<CodeSection>, content: &str) {
    if is_test_node(node) {
        let text_range = node.text_range();
        let start_offset = text_range.start().into();
        let end_offset = text_range.end().into();

        let start_line = content[..start_offset].lines().count().saturating_sub(1);
        let end_line = content[..end_offset].lines().count().saturating_sub(1);

        trace!("Found test section: lines {}-{}", start_line, end_line);

        sections.push(CodeSection {
            start_line,
            end_line,
        });
        return; // Don't recurse into test sections
    }

    for child in node.children() {
        find_test_sections(&child, sections, content);
    }
}

/// Determines which lines belong to production vs test code.
fn classify_lines(content: &str) -> Vec<bool> {
    let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
    let root = parse.syntax_node();

    let mut test_sections = Vec::new();
    find_test_sections(&root, &mut test_sections, content);

    let total_lines = content.lines().count();
    let mut is_test_line = vec![false; total_lines];

    for section in test_sections {
        for line_idx in section.start_line..=section.end_line.min(total_lines - 1) {
            is_test_line[line_idx] = true;
        }
    }

    debug!(
        "Classified {} lines: {} test, {} production",
        total_lines,
        is_test_line.iter().filter(|&&x| x).count(),
        is_test_line.iter().filter(|&&x| !x).count()
    );

    is_test_line
}

// ============================================================================
// File Analysis
// ============================================================================

/// Analyzes a single Rust source file.
fn analyze_file(path: &Path) -> Result<FileStats, String> {
    trace!("Analyzing file: {}", path.display());

    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let total_lines = content.lines().count();
    if total_lines == 0 {
        debug!("Empty file: {}", path.display());
        return Ok(FileStats {
            path: path.to_string_lossy().to_string(),
            total: LineStats {
                all_lines: 0,
                ..Default::default()
            },
            production: LineStats::default(),
            test: LineStats::default(),
        });
    }

    let line_types = analyze_lines(&content);
    let is_test_line = classify_lines(&content);

    // Compute total stats
    let total = compute_line_stats(&line_types, total_lines);

    // Compute production stats
    let prod_line_types: Vec<_> = line_types
        .iter()
        .zip(is_test_line.iter())
        .filter(|&(_, &is_test)| !is_test)
        .map(|(lt, _)| *lt)
        .collect();
    let production = compute_line_stats(&prod_line_types, prod_line_types.len());

    // Compute test stats
    let test_line_types: Vec<_> = line_types
        .iter()
        .zip(is_test_line.iter())
        .filter(|&(_, &is_test)| is_test)
        .map(|(lt, _)| *lt)
        .collect();
    let test = compute_line_stats(&test_line_types, test_line_types.len());

    debug!(
        "File {}: total={}, prod={}, test={}",
        path.display(),
        total.all_lines,
        production.all_lines,
        test.all_lines
    );

    Ok(FileStats {
        path: path.to_string_lossy().to_string(),
        total,
        production,
        test,
    })
}

/// Analyzes all Rust files in a directory recursively.
fn analyze_directory(dir: &Path) -> Result<Vec<FileStats>, String> {
    let mut file_stats = Vec::new();

    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
            file_stats.push(analyze_file(path)?);
        }
    }

    if file_stats.is_empty() {
        return Err(format!("No Rust files found in {}", dir.display()));
    }

    debug!("Analyzed {} files in {}", file_stats.len(), dir.display());

    Ok(file_stats)
}

// ============================================================================
// Output Formatting
// ============================================================================

/// Formats line statistics for plain text output.
fn format_line_stats(stats: &LineStats, indent: usize) -> String {
    let prefix = " ".repeat(indent);
    format!(
        "{}All lines: {}\n\
         {}Blank lines: {}\n\
         {}Comment lines: {}\n\
         {}Code lines: {}",
        prefix,
        stats.all_lines,
        prefix,
        stats.blank_lines,
        prefix,
        stats.comment_lines,
        prefix,
        stats.code_lines
    )
}

/// Outputs the report in plain text format.
fn output_text(report: &Report) {
    println!("Summary:");
    println!("  Files: {}", report.summary.files);
    println!("  Total:");
    println!("{}", format_line_stats(&report.summary.total, 4));
    println!("  Production:");
    println!("{}", format_line_stats(&report.summary.production, 4));
    println!("  Test:");
    println!("{}", format_line_stats(&report.summary.test, 4));

    println!("\nFiles:");
    for file in &report.files {
        println!("  {}:", file.path);
        println!("    Total:");
        println!("{}", format_line_stats(&file.total, 6));
        println!("    Production:");
        println!("{}", format_line_stats(&file.production, 6));
        println!("    Test:");
        println!("{}", format_line_stats(&file.test, 6));
    }
}

/// Outputs the report in JSON format.
fn output_json(report: &Report) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    println!("{}", json);
    Ok(())
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<(), String> {
    let args = Args::parse();

    // Initialize logger
    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Trace)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Warn)
            .init();
    }

    // Determine what to analyze
    let file_stats = if let Some(file_path) = &args.file {
        vec![analyze_file(file_path)?]
    } else if let Some(dir_path) = &args.dir {
        analyze_directory(dir_path)?
    } else {
        // No arguments provided, show help
        eprintln!("Error: Either --file or --dir must be specified.\n");
        eprintln!("Use --help for more information.");
        std::process::exit(1);
    };

    // Build summary
    let mut summary = Summary::default();
    for stats in &file_stats {
        summary.add_file(stats);
    }

    let report = Report {
        summary,
        files: file_stats,
    };

    // Output results
    match args.output_format() {
        OutputFormat::Text => output_text(&report),
        OutputFormat::Json => output_json(&report)?,
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_stats_default() {
        let stats = LineStats::default();
        assert_eq!(stats.all_lines, 0);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.code_lines, 0);
    }

    #[test]
    fn test_line_stats_add() {
        let mut stats1 = LineStats {
            all_lines: 10,
            blank_lines: 2,
            comment_lines: 3,
            code_lines: 5,
        };
        let stats2 = LineStats {
            all_lines: 20,
            blank_lines: 4,
            comment_lines: 6,
            code_lines: 10,
        };
        stats1.add(&stats2);
        assert_eq!(stats1.all_lines, 30);
        assert_eq!(stats1.blank_lines, 6);
        assert_eq!(stats1.comment_lines, 9);
        assert_eq!(stats1.code_lines, 15);
    }

    #[test]
    fn test_analyze_lines_blank() {
        let content = "\n\n  \n\t\n";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert!(line_types.iter().all(|&t| t == LineType::Blank));
    }

    #[test]
    fn test_analyze_lines_line_comments() {
        let content = "// comment 1\n// comment 2\n/// doc comment";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Comment));
    }

    #[test]
    fn test_analyze_lines_block_comment() {
        let content = "/* start\nmiddle\nend */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Comment));
    }

    #[test]
    fn test_analyze_lines_code() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Code));
    }

    #[test]
    fn test_analyze_lines_mixed() {
        let content = "// comment\n\nfn main() {}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Blank);
        assert_eq!(line_types[2], LineType::Code);
    }

    #[test]
    fn test_compute_line_stats() {
        let line_types = vec![
            LineType::Comment,
            LineType::Blank,
            LineType::Code,
            LineType::Code,
            LineType::Blank,
        ];
        let stats = compute_line_stats(&line_types, 5);
        assert_eq!(stats.all_lines, 5);
        assert_eq!(stats.blank_lines, 2);
        assert_eq!(stats.comment_lines, 1);
        assert_eq!(stats.code_lines, 2);
    }

    #[test]
    fn test_classify_lines_no_tests() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let is_test = classify_lines(content);
        assert_eq!(is_test.len(), 3);
        assert!(is_test.iter().all(|&x| !x));
    }

    #[test]
    fn test_classify_lines_with_test_function() {
        let content = r#"
fn production() {}

#[test]
fn test_something() {
    assert!(true);
}
"#;
        let is_test = classify_lines(content);
        // Lines: "", "fn production() {}", "", "#[test]", "fn test_something() {", "    assert!(true);", "}"
        assert!(is_test.len() > 0);
        // The test function lines should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    #[test]
    fn test_classify_lines_with_test_module() {
        let content = r#"
fn production() {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_it() {}
}
"#;
        let is_test = classify_lines(content);
        assert!(is_test.len() > 0);
        // The module and its contents should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    #[test]
    fn test_summary_add_file() {
        let mut summary = Summary::default();
        let file_stats = FileStats {
            path: "test.rs".to_string(),
            total: LineStats {
                all_lines: 10,
                blank_lines: 2,
                comment_lines: 3,
                code_lines: 5,
            },
            production: LineStats {
                all_lines: 7,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 4,
            },
            test: LineStats {
                all_lines: 3,
                blank_lines: 1,
                comment_lines: 1,
                code_lines: 1,
            },
        };
        summary.add_file(&file_stats);
        assert_eq!(summary.files, 1);
        assert_eq!(summary.total.all_lines, 10);
        assert_eq!(summary.production.all_lines, 7);
        assert_eq!(summary.test.all_lines, 3);
    }

    #[test]
    fn test_format_line_stats() {
        let stats = LineStats {
            all_lines: 100,
            blank_lines: 20,
            comment_lines: 30,
            code_lines: 50,
        };
        let formatted = format_line_stats(&stats, 2);
        assert!(formatted.contains("All lines: 100"));
        assert!(formatted.contains("Blank lines: 20"));
        assert!(formatted.contains("Comment lines: 30"));
        assert!(formatted.contains("Code lines: 50"));
    }

    #[test]
    fn test_empty_file_analysis() {
        let content = "";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 0);
    }

    #[test]
    fn test_analyze_lines_multiline_block_comment() {
        let content = "code line\n/* comment start\ncomment middle\ncomment end */\nmore code";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 5);
        assert_eq!(line_types[0], LineType::Code);
        assert_eq!(line_types[1], LineType::Comment);
        assert_eq!(line_types[2], LineType::Comment);
        assert_eq!(line_types[3], LineType::Comment);
        assert_eq!(line_types[4], LineType::Code);
    }
}

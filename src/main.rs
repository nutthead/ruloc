//! # ruloc - Rust Lines of Code Counter
//!
//! A minimalist tool for counting lines of code in Rust source files.
//! Provides detailed statistics including total, production, and test code metrics.

use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, trace};
use ra_ap_syntax::{AstNode, SourceFile, SyntaxNode, ast, ast::HasAttrs};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

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
    /// Adds another `LineStats` to this one, accumulating all metrics.
    ///
    /// # Arguments
    ///
    /// * `other` - The line statistics to add to this instance
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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

impl Summary {
    /// Adds file statistics to this summary, incrementing file count and accumulating metrics.
    ///
    /// # Arguments
    ///
    /// * `file_stats` - The file statistics to add to this summary
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

/// Output format for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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

    /// Maximum file size to analyze (supports units: KB, MB, GB; defaults to bytes).
    /// Examples: 1000, 3.5KB, 10MB, 1.1GB
    #[arg(long, value_name = "SIZE")]
    max_file_size: Option<String>,
}

impl Args {
    /// Parses the max file size from the command-line argument.
    ///
    /// Supports units: KB, MB, GB. Without a unit, interprets as bytes.
    ///
    /// # Returns
    ///
    /// `Some(size_in_bytes)` if specified, `None` otherwise
    ///
    /// # Errors
    ///
    /// Returns an error if the size string cannot be parsed
    fn parse_max_file_size(&self) -> Result<Option<u64>, String> {
        let Some(ref size_str) = self.max_file_size else {
            return Ok(None);
        };

        parse_file_size(size_str).map(Some)
    }
}

impl Args {
    /// Determines the output format based on command-line flags.
    ///
    /// # Returns
    ///
    /// `OutputFormat::Json` if `--out-json` is specified, otherwise `OutputFormat::Text`
    fn output_format(&self) -> OutputFormat {
        if self.out_json {
            OutputFormat::Json
        } else {
            OutputFormat::Text
        }
    }
}

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

/// Parses a file size string with optional unit suffix.
///
/// Supports units: KB, MB, GB (case-insensitive). Without a unit, interprets as bytes.
/// Allows decimal numbers (e.g., "3.5KB").
///
/// # Arguments
///
/// * `size_str` - The size string to parse (e.g., "1000", "3.5KB", "10MB")
///
/// # Returns
///
/// The size in bytes as `u64`
///
/// # Errors
///
/// Returns an error if the string cannot be parsed as a valid size
fn parse_file_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim();

    // Try to match unit suffix
    let (number_str, multiplier) =
        if let Some(pos) = size_str.to_uppercase().find(|c: char| c.is_alphabetic()) {
            let (num, unit) = size_str.split_at(pos);
            let mult = match unit.to_uppercase().as_str() {
                "KB" => 1024u64,
                "MB" => 1024u64 * 1024,
                "GB" => 1024u64 * 1024 * 1024,
                _ => {
                    return Err(format!(
                        "Invalid size unit: '{}'. Supported units: KB, MB, GB",
                        unit
                    ));
                }
            };
            (num, mult)
        } else {
            (size_str, 1u64)
        };

    // Parse the numeric part
    let number: f64 = number_str
        .trim()
        .parse()
        .map_err(|_| format!("Invalid size number: '{}'", number_str))?;

    if number < 0.0 {
        return Err("File size cannot be negative".to_string());
    }

    let bytes = (number * multiplier as f64) as u64;
    Ok(bytes)
}

/// Entry point for the ruloc CLI application.
///
/// Parses command-line arguments, initializes logging, analyzes the specified
/// file or directory, and outputs the results in the requested format.
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` with an error message on failure
///
/// # Errors
///
/// Returns an error if:
/// - Neither `--file` nor `--dir` is specified
/// - File reading fails
/// - Directory contains no Rust files
/// - JSON serialization fails
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

    // Parse max file size if specified
    let max_file_size = args.parse_max_file_size()?;

    // Determine what to analyze
    let file_stats = if let Some(file_path) = &args.file {
        vec![analyze_file(file_path, max_file_size)?]
    } else if let Some(dir_path) = &args.dir {
        analyze_directory(dir_path, max_file_size)?
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

/// Analyzes the content of source code to classify each line as blank, comment, or code.
///
/// Handles both line comments (`//`) and block comments (`/* */`), tracking
/// multiline block comment state across lines.
///
/// # Arguments
///
/// * `content` - The source code content to analyze
///
/// # Returns
///
/// A vector of `LineType` values, one for each line in the content
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

/// Computes line statistics from classified line types by counting occurrences.
///
/// # Arguments
///
/// * `line_types` - Slice of classified line types to count
/// * `total_lines` - Total number of lines (used for the `all_lines` field)
///
/// # Returns
///
/// A `LineStats` instance with counts for each line type
fn compute_line_stats(line_types: &[LineType], total_lines: usize) -> LineStats {
    let blank_lines = line_types.iter().filter(|&&t| t == LineType::Blank).count();
    let comment_lines = line_types
        .iter()
        .filter(|&&t| t == LineType::Comment)
        .count();
    let code_lines = line_types.iter().filter(|&&t| t == LineType::Code).count();

    LineStats {
        all_lines: total_lines,
        blank_lines,
        comment_lines,
        code_lines,
    }
}

/// Represents a code section with its classification and line range.
#[derive(Debug, Clone)]
struct CodeSection {
    /// Starting line number (0-indexed).
    start_line: usize,
    /// Ending line number (0-indexed, inclusive).
    end_line: usize,
}

/// Determines if a syntax node represents a test item by checking for test attributes.
///
/// Identifies functions with `#[test]` or `#[cfg(test)]` attributes, and modules
/// with `#[cfg(test)]` attributes.
///
/// # Arguments
///
/// * `node` - The syntax tree node to examine
///
/// # Returns
///
/// `true` if the node represents a test function or test module, `false` otherwise
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

/// Recursively finds test sections in the syntax tree by traversing AST nodes.
///
/// When a test node is found, adds its line range to the sections vector and
/// stops recursing into that subtree (since nested tests are part of the parent).
///
/// # Arguments
///
/// * `node` - The current syntax tree node being examined
/// * `sections` - Mutable vector to collect discovered test sections
/// * `content` - The complete source file content (used for line offset calculation)
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

/// Determines which lines belong to production vs test code using AST analysis.
///
/// Parses the source code to build a syntax tree, identifies all test sections,
/// and marks their corresponding line ranges as test code.
///
/// # Arguments
///
/// * `content` - The source code content to classify
///
/// # Returns
///
/// A vector of boolean values, one per line, where `true` indicates test code
/// and `false` indicates production code
fn classify_lines(content: &str) -> Vec<bool> {
    let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
    let root = parse.syntax_node();

    let mut test_sections = Vec::new();
    find_test_sections(&root, &mut test_sections, content);

    let total_lines = content.lines().count();
    let mut is_test_line = vec![false; total_lines];

    for section in test_sections {
        let end = section.end_line.min(total_lines - 1);
        is_test_line[section.start_line..=end].fill(true);
    }

    debug!(
        "Classified {} lines: {} test, {} production",
        total_lines,
        is_test_line.iter().filter(|&&x| x).count(),
        is_test_line.iter().filter(|&&x| !x).count()
    );

    is_test_line
}

/// Analyzes a single Rust source file to compute line statistics.
///
/// Reads the file, classifies lines as blank/comment/code, identifies test sections,
/// and computes separate statistics for total, production, and test code.
///
/// # Arguments
///
/// * `path` - Path to the Rust source file to analyze
/// * `max_file_size` - Optional maximum file size in bytes; files larger are skipped
///
/// # Returns
///
/// `Ok(FileStats)` with the analysis results, or `Err(String)` if file reading fails
/// or the file exceeds the size limit
///
/// # Errors
///
/// Returns an error if the file cannot be read or exceeds the maximum size
fn analyze_file(path: &Path, max_file_size: Option<u64>) -> Result<FileStats, String> {
    trace!("Analyzing file: {}", path.display());

    // Check file size if limit is specified
    if let Some(max_size) = max_file_size {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Failed to get metadata for {}: {}", path.display(), e))?;
        let file_size = metadata.len();

        if file_size > max_size {
            debug!(
                "Skipping file {} (size: {} bytes exceeds limit: {} bytes)",
                path.display(),
                file_size,
                max_size
            );
            return Err(format!(
                "File {} exceeds maximum size ({} > {} bytes)",
                path.display(),
                file_size,
                max_size
            ));
        }
    }

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

/// Analyzes all Rust files in a directory recursively using parallel directory traversal.
///
/// Walks the directory tree, identifies all `.rs` files, and analyzes each one in parallel
/// using rayon. Follows symbolic links during traversal. Files exceeding the size limit
/// are skipped. Shows a progress bar during processing.
///
/// # Arguments
///
/// * `dir` - Path to the directory to analyze
/// * `max_file_size` - Optional maximum file size in bytes; larger files are skipped
///
/// # Returns
///
/// `Ok(Vec<FileStats>)` containing statistics for all analyzed files, or
/// `Err(String)` if no Rust files are found or analysis fails
///
/// # Errors
///
/// Returns an error if:
/// - No Rust files are found in the directory
/// - Any individual file analysis fails (except for size limit violations, which are skipped)
fn analyze_directory(dir: &Path, max_file_size: Option<u64>) -> Result<Vec<FileStats>, String> {
    // First pass: collect all .rs file paths
    let rust_files: Vec<PathBuf> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
        .map(|e| e.path().to_path_buf())
        .collect();

    if rust_files.is_empty() {
        return Err(format!("No Rust files found in {}", dir.display()));
    }

    // Setup progress bar
    let progress = ProgressBar::new(rust_files.len() as u64);
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓░"),
    );

    // Atomic counter for skipped files
    let skipped_count = Arc::new(AtomicUsize::new(0));

    // Second pass: analyze files in parallel
    let file_stats: Vec<FileStats> = rust_files
        .par_iter()
        .filter_map(|path| {
            let result = analyze_file(path, max_file_size);
            progress.inc(1);

            match result {
                Ok(stats) => Some(stats),
                Err(e) if e.contains("exceeds maximum size") => {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    debug!("Skipped: {}", e);
                    None
                }
                Err(e) => {
                    progress.println(format!("Error: {}", e));
                    None
                }
            }
        })
        .collect();

    progress.finish_with_message("Analysis complete");

    let final_skipped = skipped_count.load(Ordering::Relaxed);
    debug!(
        "Analyzed {} files in {} (skipped {} files exceeding size limit)",
        file_stats.len(),
        dir.display(),
        final_skipped
    );

    if file_stats.is_empty() {
        return Err(format!(
            "No Rust files could be analyzed in {}",
            dir.display()
        ));
    }

    Ok(file_stats)
}

/// Formats line statistics for plain text output with proper indentation.
///
/// # Arguments
///
/// * `stats` - The line statistics to format
/// * `indent` - Number of spaces to indent each line
///
/// # Returns
///
/// A formatted string with all line counts displayed on separate lines
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

/// Outputs the report in plain text format to stdout.
///
/// Displays a summary section with aggregated statistics, followed by
/// detailed statistics for each analyzed file.
///
/// # Arguments
///
/// * `report` - The analysis report to output
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

/// Outputs the report in JSON format to stdout.
///
/// Serializes the report to pretty-printed JSON using serde.
///
/// # Arguments
///
/// * `report` - The analysis report to output
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` if JSON serialization fails
///
/// # Errors
///
/// Returns an error if the report cannot be serialized to JSON
fn output_json(report: &Report) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    println!("{}", json);
    Ok(())
}

/// Unit tests for the ruloc line counting and analysis functionality.
///
/// Tests cover:
/// - Line statistics operations (default, add)
/// - Line classification (blank, comments, code)
/// - Block comment handling
/// - Production vs test code classification
/// - Summary aggregation
/// - Output formatting
#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that `LineStats::default()` creates a zero-initialized instance.
    #[test]
    fn test_line_stats_default() {
        let stats = LineStats::default();
        assert_eq!(stats.all_lines, 0);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.code_lines, 0);
    }

    /// Tests that `LineStats::add()` correctly accumulates statistics.
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

    /// Tests that blank lines (empty or whitespace-only) are correctly identified.
    #[test]
    fn test_analyze_lines_blank() {
        let content = "\n\n  \n\t\n";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert!(line_types.iter().all(|&t| t == LineType::Blank));
    }

    /// Tests that line comments (`//`) and doc comments (`///`) are correctly identified.
    #[test]
    fn test_analyze_lines_line_comments() {
        let content = "// comment 1\n// comment 2\n/// doc comment";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Comment));
    }

    /// Tests that multiline block comments (`/* ... */`) are correctly identified.
    #[test]
    fn test_analyze_lines_block_comment() {
        let content = "/* start\nmiddle\nend */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Comment));
    }

    /// Tests that code lines are correctly identified.
    #[test]
    fn test_analyze_lines_code() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Code));
    }

    /// Tests classification of mixed content (comments, blanks, and code).
    #[test]
    fn test_analyze_lines_mixed() {
        let content = "// comment\n\nfn main() {}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Blank);
        assert_eq!(line_types[2], LineType::Code);
    }

    /// Tests that line statistics are correctly computed from line type classifications.
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

    /// Tests that production code without tests is classified as non-test.
    #[test]
    fn test_classify_lines_no_tests() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let is_test = classify_lines(content);
        assert_eq!(is_test.len(), 3);
        assert!(is_test.iter().all(|&x| !x));
    }

    /// Tests that functions marked with `#[test]` are correctly identified as test code.
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
        assert!(!is_test.is_empty());
        // The test function lines should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    /// Tests that modules marked with `#[cfg(test)]` are correctly identified as test code.
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
        assert!(!is_test.is_empty());
        // The module and its contents should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    /// Tests that `Summary::add_file()` correctly aggregates file statistics.
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

    /// Tests that line statistics are correctly formatted for text output.
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

    /// Tests that empty files (with no content) are handled correctly.
    #[test]
    fn test_empty_file_analysis() {
        let content = "";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 0);
    }

    /// Tests that multiline block comments spanning multiple lines are correctly handled.
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

    /// Tests parsing file size with no unit (bytes).
    #[test]
    fn test_parse_file_size_bytes() {
        assert_eq!(parse_file_size("1000").unwrap(), 1000);
        assert_eq!(parse_file_size("500").unwrap(), 500);
        assert_eq!(parse_file_size("1").unwrap(), 1);
    }

    /// Tests parsing file size with KB unit.
    #[test]
    fn test_parse_file_size_kb() {
        assert_eq!(parse_file_size("1KB").unwrap(), 1024);
        assert_eq!(parse_file_size("1kb").unwrap(), 1024);
        assert_eq!(parse_file_size("3.5KB").unwrap(), 3584);
        assert_eq!(parse_file_size("10KB").unwrap(), 10240);
    }

    /// Tests parsing file size with MB unit.
    #[test]
    fn test_parse_file_size_mb() {
        assert_eq!(parse_file_size("1MB").unwrap(), 1048576);
        assert_eq!(parse_file_size("1mb").unwrap(), 1048576);
        assert_eq!(parse_file_size("2.5MB").unwrap(), 2621440);
    }

    /// Tests parsing file size with GB unit.
    #[test]
    fn test_parse_file_size_gb() {
        assert_eq!(parse_file_size("1GB").unwrap(), 1073741824);
        assert_eq!(parse_file_size("1gb").unwrap(), 1073741824);
        assert_eq!(parse_file_size("1.1GB").unwrap(), 1181116006);
    }

    /// Tests parsing file size with whitespace.
    #[test]
    fn test_parse_file_size_with_whitespace() {
        assert_eq!(parse_file_size("  1000  ").unwrap(), 1000);
        assert_eq!(parse_file_size("  3.5KB  ").unwrap(), 3584);
    }

    /// Tests parsing invalid file size returns error.
    #[test]
    fn test_parse_file_size_invalid() {
        assert!(parse_file_size("invalid").is_err());
        assert!(parse_file_size("").is_err());
        assert!(parse_file_size("KB").is_err());
        assert!(parse_file_size("1TB").is_err()); // Unsupported unit
    }

    /// Tests parsing negative file size returns error.
    #[test]
    fn test_parse_file_size_negative() {
        assert!(parse_file_size("-100").is_err());
        assert!(parse_file_size("-1KB").is_err());
    }

    /// Tests Args::parse_max_file_size with valid input.
    #[test]
    fn test_args_parse_max_file_size_some() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            verbose: false,
            max_file_size: Some("10MB".to_string()),
        };
        let result = args.parse_max_file_size().unwrap();
        assert_eq!(result, Some(10 * 1024 * 1024));
    }

    /// Tests Args::parse_max_file_size with None.
    #[test]
    fn test_args_parse_max_file_size_none() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            verbose: false,
            max_file_size: None,
        };
        let result = args.parse_max_file_size().unwrap();
        assert_eq!(result, None);
    }

    /// Tests Args::output_format returns Json when flag is set.
    #[test]
    fn test_args_output_format_json() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: true,
            verbose: false,
            max_file_size: None,
        };
        assert_eq!(args.output_format(), OutputFormat::Json);
    }

    /// Tests Args::output_format returns Text by default.
    #[test]
    fn test_args_output_format_text() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            verbose: false,
            max_file_size: None,
        };
        assert_eq!(args.output_format(), OutputFormat::Text);
    }

    /// Tests analyze_file with a real Rust file.
    #[test]
    fn test_analyze_file_integration() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_ruloc.rs");

        let test_code = r#"
// Production code
fn hello() {
    println!("hello");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_hello() {
        assert!(true);
    }
}
"#;

        std::fs::write(&temp_file, test_code).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.all_lines > 0);
        assert!(stats.production.code_lines > 0);
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_file respects max_file_size limit.
    #[test]
    fn test_analyze_file_size_limit() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_ruloc_large.rs");

        let test_code = "// A large file\n".repeat(100);
        std::fs::write(&temp_file, &test_code).unwrap();

        // File is ~1600 bytes, set limit to 100 bytes
        let result = analyze_file(&temp_file, Some(100));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum size"));

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_text formatting.
    #[test]
    fn test_output_text() {
        let report = Report {
            summary: Summary {
                files: 1,
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
            },
            files: vec![],
        };

        output_text(&report);
        // Just ensure it doesn't panic
    }

    /// Tests output_json formatting.
    #[test]
    fn test_output_json() {
        let report = Report {
            summary: Summary {
                files: 1,
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
            },
            files: vec![],
        };

        let result = output_json(&report);
        assert!(result.is_ok());
    }

    /// Tests analyze_directory with a temporary directory structure.
    #[test]
    fn test_analyze_directory_integration() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_dir");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create test files
        let file1 = temp_dir.join("file1.rs");
        fs::write(&file1, "fn main() {}\n").unwrap();

        let file2 = temp_dir.join("file2.rs");
        fs::write(&file2, "#[test]\nfn test() {}\n").unwrap();

        let result = analyze_directory(&temp_dir, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.len(), 2);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_directory with max_file_size filtering.
    #[test]
    fn test_analyze_directory_with_size_filter() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_dir_filter");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a small file
        let small_file = temp_dir.join("small.rs");
        fs::write(&small_file, "fn f() {}\n").unwrap();

        // Create a large file
        let large_file = temp_dir.join("large.rs");
        fs::write(&large_file, "// Large\n".repeat(100)).unwrap();

        // Set size limit to 100 bytes - should skip the large file
        let result = analyze_directory(&temp_dir, Some(100));
        assert!(result.is_ok());

        let stats = result.unwrap();
        // Only the small file should be analyzed
        assert!(stats.len() <= 1);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_file with invalid Rust code.
    #[test]
    fn test_analyze_file_invalid_rust() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_invalid.rs");

        // This is syntactically invalid but should still count lines
        let invalid_code = "fn broken( {}\nthis is not rust\n";
        std::fs::write(&temp_file, invalid_code).unwrap();

        let result = analyze_file(&temp_file, None);
        // Should succeed even with invalid syntax, just counts lines
        assert!(result.is_ok());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_file with a file that has complex nested test modules.
    #[test]
    fn test_analyze_file_nested_tests() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_nested.rs");

        let test_code = r#"
fn prod() {}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    mod nested {
        #[test]
        fn inner_test() {}
    }
}
"#;

        std::fs::write(&temp_file, test_code).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_text with detailed file statistics.
    #[test]
    fn test_output_text_with_files() {
        let report = Report {
            summary: Summary {
                files: 1,
                total: LineStats {
                    all_lines: 15,
                    blank_lines: 3,
                    comment_lines: 4,
                    code_lines: 8,
                },
                production: LineStats {
                    all_lines: 10,
                    blank_lines: 2,
                    comment_lines: 3,
                    code_lines: 5,
                },
                test: LineStats {
                    all_lines: 5,
                    blank_lines: 1,
                    comment_lines: 1,
                    code_lines: 3,
                },
            },
            files: vec![FileStats {
                path: "test.rs".to_string(),
                total: LineStats {
                    all_lines: 15,
                    blank_lines: 3,
                    comment_lines: 4,
                    code_lines: 8,
                },
                production: LineStats {
                    all_lines: 10,
                    blank_lines: 2,
                    comment_lines: 3,
                    code_lines: 5,
                },
                test: LineStats {
                    all_lines: 5,
                    blank_lines: 1,
                    comment_lines: 1,
                    code_lines: 3,
                },
            }],
        };

        output_text(&report);
        // Just ensure it doesn't panic with file details
    }
}

use super::screenshot::capture_raw;
use crate::output::CmdError;
use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct OcrMatch {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

struct Word {
    /// Tesseract's (block, paragraph, line) numbers.
    line: (u32, u32, u32),
    text: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn missing(detail: impl std::fmt::Display) -> CmdError {
    CmdError::unsupported(format!(
        "OCR needs the tesseract command plus at least one language pack (e.g. tesseract-data-eng): {detail}"
    ))
}

/// English if it's installed, otherwise whichever language pack is.
fn language() -> Result<String, CmdError> {
    let output = Command::new("tesseract").arg("--list-langs").output().map_err(missing)?;
    let listing = String::from_utf8_lossy(&output.stdout);
    // The first line is a header; "osd" is script detection, not a language.
    let langs: Vec<&str> = listing.lines().skip(1).map(str::trim).filter(|l| !l.is_empty() && *l != "osd").collect();
    if langs.contains(&"eng") {
        return Ok("eng".to_string());
    }
    langs.first().map(|l| l.to_string()).ok_or_else(|| missing("no language pack is installed"))
}

/// Parses tesseract's TSV output: level, page, block, paragraph, line, word,
/// left, top, width, height, confidence, text. Level 5 rows are words.
fn parse_words(tsv: &str) -> Vec<Word> {
    tsv.lines()
        .filter_map(|row| {
            let cols: Vec<&str> = row.splitn(12, '\t').collect();
            if cols.len() != 12 || cols[0] != "5" || cols[11].trim().is_empty() {
                return None;
            }
            let num = |i: usize| cols[i].parse::<i32>().ok();
            Some(Word {
                line: (cols[2].parse().ok()?, cols[3].parse().ok()?, cols[4].parse().ok()?),
                text: cols[11].trim().to_string(),
                x: num(6)?,
                y: num(7)?,
                w: num(8)?,
                h: num(9)?,
            })
        })
        .collect()
}

/// The shortest run of consecutive words on one line whose text contains
/// `needle`, compared without regard to case.
fn best_run<'a>(words: &'a [Word], needle: &str) -> Option<&'a [Word]> {
    let needle_lower = needle.to_lowercase();
    let mut best: Option<&[Word]> = None;
    for start in 0..words.len() {
        for end in start..words.len() {
            if words[end].line != words[start].line {
                break;
            }
            let run = &words[start..=end];
            let text = run.iter().map(|w| w.text.to_lowercase()).collect::<Vec<_>>().join(" ");
            if text.contains(&needle_lower) {
                if best.is_none_or(|b| run.len() < b.len()) {
                    best = Some(run);
                }
                break;
            }
        }
    }
    best
}

/// The union of a run of words, in absolute desktop pixels.
fn matched(run: &[Word], origin: (i32, i32)) -> OcrMatch {
    let left = run.iter().map(|w| w.x).min().unwrap_or(0);
    let top = run.iter().map(|w| w.y).min().unwrap_or(0);
    let right = run.iter().map(|w| w.x + w.w).max().unwrap_or(0);
    let bottom = run.iter().map(|w| w.y + w.h).max().unwrap_or(0);
    OcrMatch {
        text: run.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
        x: origin.0 + left,
        y: origin.1 + top,
        w: right - left,
        h: bottom - top,
    }
}

/// Reads the words in an area with Tesseract, plus the area's origin.
fn recognize(
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
) -> Result<(Vec<Word>, (i32, i32)), CmdError> {
    let lang = language()?;
    let (img, origin_x, origin_y) = capture_raw(monitor, window, region)?;

    let path = std::env::temp_dir().join(format!("guictl-ocr-{}.png", std::process::id()));
    img.save(&path).map_err(|e| CmdError::os(format!("could not write the OCR input image: {e}")))?;
    let output = Command::new("tesseract").arg(&path).args(["stdout", "-l", &lang, "tsv"]).output();
    let _ = std::fs::remove_file(&path);
    let output = output.map_err(missing)?;
    if !output.status.success() {
        return Err(CmdError::os(format!(
            "tesseract failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok((parse_words(&String::from_utf8_lossy(&output.stdout)), (origin_x, origin_y)))
}

/// Finds `needle` on screen and returns its bounding box in absolute desktop
/// pixels: the union of the shortest run of consecutive words on one line
/// whose text contains `needle`.
pub fn find(
    needle: &str,
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
) -> Result<OcrMatch, CmdError> {
    let (words, origin) = recognize(monitor, window, region)?;
    let run = best_run(&words, needle)
        .ok_or_else(|| CmdError::not_found(format!("no text matching '{needle}' found on screen")))?;
    Ok(matched(run, origin))
}

/// Every line of text in the area, with its bounding box in absolute desktop
/// pixels.
pub fn read(monitor: Option<&str>, window: Option<&str>, region: Option<&str>) -> Result<Vec<OcrMatch>, CmdError> {
    let (words, origin) = recognize(monitor, window, region)?;
    Ok(words.chunk_by(|a, b| a.line == b.line).map(|line| matched(line, origin)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // level page block par line word left top width height conf text
    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
4\t1\t1\t1\t1\t0\t10\t10\t200\t20\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t10\t40\t20\t96\tSave\n\
5\t1\t1\t1\t1\t2\t60\t12\t30\t18\t95\tAs\n\
5\t1\t1\t1\t1\t3\t100\t10\t50\t20\t95\tCopy\n\
5\t1\t1\t1\t2\t1\t10\t40\t40\t20\t90\tSave\n\
5\t1\t1\t1\t2\t2\t60\t40\t20\t20\t11\t \n";

    #[test]
    fn parses_only_word_rows_with_text() {
        let words = parse_words(TSV);
        assert_eq!(words.len(), 4);
        assert_eq!(words[1].text, "As");
        assert_eq!((words[1].x, words[1].y, words[1].w, words[1].h), (60, 12, 30, 18));
        assert_eq!(words[3].line, (1, 1, 2));
    }

    #[test]
    fn matches_the_shortest_run_ignoring_case() {
        let words = parse_words(TSV);
        let run = best_run(&words, "save as").unwrap();
        assert_eq!(run.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Save", "As"]);
        let run = best_run(&words, "COPY").unwrap();
        assert_eq!(run.len(), 1);
        assert_eq!(run[0].x, 100);
    }

    #[test]
    fn lines_are_the_union_of_their_words() {
        let words = parse_words(TSV);
        let lines: Vec<OcrMatch> =
            words.chunk_by(|a, b| a.line == b.line).map(|line| matched(line, (1000, 0))).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Save As Copy");
        assert_eq!((lines[0].x, lines[0].y, lines[0].w, lines[0].h), (1010, 10, 140, 20));
        assert_eq!(lines[1].text, "Save");
    }

    #[test]
    fn a_match_never_spans_two_lines() {
        let words = parse_words(TSV);
        // "Copy" ends line 1 and "Save" starts line 2.
        assert!(best_run(&words, "copy save").is_none());
        assert!(best_run(&words, "missing").is_none());
    }
}

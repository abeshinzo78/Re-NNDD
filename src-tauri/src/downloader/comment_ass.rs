//! ニコニコ風コメントを ASS (Advanced SubStation Alpha) 字幕へ変換するコア。
//!
//! Phase 1.9「コメント焼き込みエクスポート」の心臓部。プレイヤーが
//! `@xpadev-net/niconicomments` で Canvas にリアルタイム描画しているものを、
//! ffmpeg の `ass` フィルタで焼き込めるように **静的な ASS** へ落とし込む。
//!
//! 設計方針 (README「Rust コア重視」):
//! - ここは I/O も ffmpeg も触らない純粋ロジック。文字列 ASS を返すだけなので
//!   単体テストで全分岐を固定できる。
//! - レイアウトは「レーン (行) グリッド + 時間衝突判定」という danmaku→ASS
//!   変換で実績のある方式を採用。流れる (naka) コメントは追突しないレーンへ、
//!   固定 (ue/shita) コメントは時間が被らないレーンへ割り当てる。
//!
//! niconicomments の html5 既定フォントサイズ (1080 ステージ基準) に合わせて
//! small=47 / medium=74 / big=110 px を採用し、実際の動画高さへ線形スケール
//! する。これでプレイヤー表示と焼き込み結果の見た目を揃える。

/// niconico の基準ステージ高さ。フォントサイズはこの高さでの px を実動画高さへ
/// スケールする。
const NICO_STAGE_HEIGHT: f64 = 1080.0;
const FONT_SMALL_AT_1080: f64 = 47.0;
const FONT_MEDIUM_AT_1080: f64 = 74.0;
const FONT_BIG_AT_1080: f64 = 110.0;

/// 焼き込み対象 1 コメント。DB / API どちらの形からでも詰められる薄い構造体。
#[derive(Debug, Clone)]
pub struct BurnInComment {
    /// 再生位置 (ミリ秒)。
    pub vpos_ms: i64,
    /// 本文。
    pub content: String,
    /// niconico コマンド列 (mail をスペース分割したもの)。色・位置・サイズ。
    pub commands: Vec<String>,
    /// 投稿者コメントか。色未指定時の既定色決定に使う。
    pub is_owner: bool,
}

/// ASS 生成オプション。UI から調整できる項目はここに集約する。
#[derive(Debug, Clone)]
pub struct AssOptions {
    /// 出力解像度 (= 入力動画解像度)。
    pub width: u32,
    pub height: u32,
    /// 動画長 (秒)。これを超えて出現するコメントは描かない。
    pub duration_sec: f64,
    /// フォント倍率。1.0 で niconico 標準相当。
    pub font_scale: f64,
    /// 不透明度 0.0〜1.0。1.0 で完全不透明。
    pub opacity: f64,
    /// 流れるコメントが画面を横断する秒数 (niconico 標準は 4 秒)。
    pub scroll_duration_sec: f64,
    /// 固定コメント (ue/shita) の表示秒数 (niconico 標準は 3 秒)。
    pub fixed_duration_sec: f64,
    /// libass に渡すフォント名。fontconfig が CJK へフォールバックする。
    pub font_name: String,
}

impl Default for AssOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            duration_sec: 0.0,
            font_scale: 1.0,
            opacity: 1.0,
            scroll_duration_sec: 4.0,
            fixed_duration_sec: 3.0,
            font_name: "sans-serif".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Naka,
    Ue,
    Shita,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    Small,
    Medium,
    Big,
}

/// コマンド列を解釈した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedStyle {
    position: Position,
    size: Size,
    /// 0xRRGGBB。
    rgb: u32,
    /// `invisible` 指定。描画をスキップする。
    invisible: bool,
}

/// niconico の名前付き色 → 0xRRGGBB。無名は None。
fn named_color(name: &str) -> Option<u32> {
    let c = match name {
        // 通常色
        "white" => 0xFFFFFF,
        "red" => 0xFF0000,
        "pink" => 0xFF8080,
        "orange" => 0xFFC000,
        "yellow" => 0xFFFF00,
        "green" => 0x00FF00,
        "cyan" => 0x00FFFF,
        "blue" => 0x0000FF,
        "purple" => 0xC000FF,
        "black" => 0x000000,
        // プレミアム色 (2 系列・別名含む)
        "white2" | "niconicowhite" => 0xCCCC99,
        "red2" | "truered" => 0xCC0033,
        "pink2" => 0xFF33CC,
        "orange2" | "passionorange" => 0xFF6600,
        "yellow2" | "madyellow" => 0x999900,
        "green2" | "elementalgreen" => 0x00CC66,
        "cyan2" => 0x00CCCC,
        "blue2" | "marineblue" => 0x3399FF,
        "purple2" | "nobleviolet" => 0x6633CC,
        "black2" | "niconicoblack" => 0x666666,
        _ => return None,
    };
    Some(c)
}

/// `#RRGGBB` / `#RGB` 形式をパース。
fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok(),
        3 => {
            // #RGB → #RRGGBB
            let mut chars = hex.chars();
            let r = chars.next()?;
            let g = chars.next()?;
            let b = chars.next()?;
            let expanded: String = [r, r, g, g, b, b].iter().collect();
            u32::from_str_radix(&expanded, 16).ok()
        }
        _ => None,
    }
}

fn parse_style(commands: &[String], is_owner: bool) -> ParsedStyle {
    // 投稿者コメントの既定色は白だが、視認性のため通常コメントと同じ白で扱う。
    let mut style = ParsedStyle {
        position: Position::Naka,
        size: Size::Medium,
        rgb: 0xFFFFFF,
        invisible: false,
    };
    // 色は「最後に出現したもの勝ち」。位置・サイズも同様。
    for raw in commands {
        let cmd = raw.trim();
        if cmd.is_empty() {
            continue;
        }
        let lower = cmd.to_ascii_lowercase();
        match lower.as_str() {
            "ue" => style.position = Position::Ue,
            "shita" => style.position = Position::Shita,
            "naka" => style.position = Position::Naka,
            "big" => style.size = Size::Big,
            "small" => style.size = Size::Small,
            "medium" => style.size = Size::Medium,
            "invisible" => style.invisible = true,
            _ => {
                if let Some(rgb) = named_color(&lower) {
                    style.rgb = rgb;
                } else if let Some(rgb) = parse_hex_color(cmd) {
                    style.rgb = rgb;
                }
                // 未知コマンド (184/full/patissier/device/フォント指定 等) は無視。
            }
        }
    }
    let _ = is_owner; // 既定色決定に将来使う余地を残す。
    style
}

fn font_px(size: Size, opts: &AssOptions) -> f64 {
    let base = match size {
        Size::Small => FONT_SMALL_AT_1080,
        Size::Medium => FONT_MEDIUM_AT_1080,
        Size::Big => FONT_BIG_AT_1080,
    };
    base * (opts.height as f64 / NICO_STAGE_HEIGHT) * opts.font_scale
}

/// 文字 1 つの送り幅 (フォント px に対する比率)。半角は約 0.6、それ以外 (全角・
/// CJK・絵文字) は 1.0 とみなす。衝突判定とスクロール終点に使う近似。
fn char_advance_ratio(ch: char) -> f64 {
    let code = ch as u32;
    // ASCII 印字可能 + 半角カナ域はおおむね半角。
    if (0x20..0x7F).contains(&code) || (0xFF61..=0xFF9F).contains(&code) {
        0.6
    } else {
        1.0
    }
}

fn estimate_line_width(line: &str, font_size: f64) -> f64 {
    line.chars()
        .map(|c| char_advance_ratio(c) * font_size)
        .sum()
}

/// ASS の Dialogue 本文へ安全に埋め込めるよう整形する。
/// danmaku2ass と同じく `\` `{` `}` をエスケープし、改行を `\N` にする。
/// libass が行頭スペースを詰めるので `\h` (ハードスペース) に置換する。
fn ass_escape(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        let mut escaped = String::with_capacity(raw_line.len());
        for ch in raw_line.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                '{' => escaped.push_str("\\{"),
                '}' => escaped.push_str("\\}"),
                '\r' => {} // CR は捨てる
                '\t' => escaped.push_str("\\h\\h\\h\\h"),
                _ => escaped.push(ch),
            }
        }
        // 行頭の半角スペースを \h へ (libass の leading-space 詰め対策)。
        let trimmed_len = escaped.len() - escaped.trim_start_matches(' ').len();
        if trimmed_len > 0 {
            let rest = escaped[trimmed_len..].to_string();
            escaped = "\\h".repeat(trimmed_len) + &rest;
        }
        lines.push(escaped);
    }
    lines.join("\\N")
}

/// 0xRRGGBB → ASS の `&HBBGGRR&` (BGR 順)。
fn ass_color(rgb: u32) -> String {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    format!("&H{b:02X}{g:02X}{r:02X}&")
}

/// 不透明度 (0..1) → ASS のアルファバイト 2 桁 (00=不透明, FF=透明)。
fn alpha_hex(opacity: f64) -> String {
    let clamped = opacity.clamp(0.0, 1.0);
    let a = ((1.0 - clamped) * 255.0).round() as u32;
    format!("{a:02X}")
}

/// 秒 → ASS タイムコード `H:MM:SS.cs` (センチ秒)。
fn fmt_time(sec: f64) -> String {
    let total_cs = (sec.max(0.0) * 100.0).round() as i64;
    let cs = total_cs % 100;
    let total_s = total_cs / 100;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

/// 輝度から縁取り色を決める。明るい文字は黒縁、暗い文字は白縁。
fn outline_color_for(rgb: u32) -> u32 {
    let r = ((rgb >> 16) & 0xFF) as f64;
    let g = ((rgb >> 8) & 0xFF) as f64;
    let b = (rgb & 0xFF) as f64;
    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
    if luma < 48.0 {
        0xFFFFFF
    } else {
        0x000000
    }
}

/// 流れるコメント用のレーン占有状況。各レーンに「最後に置いたコメントの
/// (出現時刻, 幅)」を保持する。
struct ScrollLanes {
    lanes: Vec<Option<(f64, f64)>>,
    width: f64,
    duration: f64,
}

impl ScrollLanes {
    fn new(count: usize, width: f64, duration: f64) -> Self {
        Self {
            lanes: vec![None; count],
            width,
            duration,
        }
    }

    /// レーン `i` が時刻 `b`・幅 `wb` の新コメントを衝突なく置けるか。
    fn lane_free(&self, i: usize, b: f64, wb: f64) -> bool {
        match self.lanes[i] {
            None => true,
            Some((a, wa)) => {
                let w = self.width;
                let d = self.duration;
                // 条件1: 既存の末尾が画面に入りきってから新コメントが出る。
                let cond1 = (b - a) >= d * wa / (w + wa);
                // 条件2: 新コメントが既存に追突しない (長い=速いので注意)。
                let cond2 = (b - a) >= d * wb / (w + wb);
                cond1 && cond2
            }
        }
    }

    /// `need` レーン連続で空いている最小開始インデックスを探す。
    /// 無ければ「最も古い (=被りが最小)」窓を返す。
    fn allocate(&mut self, b: f64, wb: f64, need: usize) -> usize {
        let need = need.max(1).min(self.lanes.len().max(1));
        let max_start = self.lanes.len().saturating_sub(need);
        // まず完全に空いている窓を探す。
        for r in 0..=max_start {
            if (r..r + need).all(|i| self.lane_free(i, b, wb)) {
                self.mark(r, need, b, wb);
                return r;
            }
        }
        // 無ければ「窓内で最後に置かれた時刻が最も古い」場所へ (被り最小化)。
        let mut best_r = 0;
        let mut best_key = f64::INFINITY;
        for r in 0..=max_start {
            let key = (r..r + need)
                .map(|i| self.lanes[i].map(|(a, _)| a).unwrap_or(f64::NEG_INFINITY))
                .fold(f64::NEG_INFINITY, f64::max);
            if key < best_key {
                best_key = key;
                best_r = r;
            }
        }
        self.mark(best_r, need, b, wb);
        best_r
    }

    fn mark(&mut self, r: usize, need: usize, b: f64, wb: f64) {
        for i in r..(r + need).min(self.lanes.len()) {
            self.lanes[i] = Some((b, wb));
        }
    }
}

/// 固定コメント用のレーン占有。各レーンに「解放時刻」を持つ。
struct FixedLanes {
    end_times: Vec<f64>,
}

impl FixedLanes {
    fn new(count: usize) -> Self {
        Self {
            end_times: vec![f64::NEG_INFINITY; count],
        }
    }

    fn allocate(&mut self, start: f64, end: f64, need: usize) -> usize {
        let need = need.max(1).min(self.end_times.len().max(1));
        let max_start = self.end_times.len().saturating_sub(need);
        for r in 0..=max_start {
            if (r..r + need).all(|i| self.end_times[i] <= start) {
                self.mark(r, need, end);
                return r;
            }
        }
        // 全部埋まっていれば最速で空く窓へ。
        let mut best_r = 0;
        let mut best_key = f64::INFINITY;
        for r in 0..=max_start {
            let key = (r..r + need)
                .map(|i| self.end_times[i])
                .fold(f64::NEG_INFINITY, f64::max);
            if key < best_key {
                best_key = key;
                best_r = r;
            }
        }
        self.mark(best_r, need, end);
        best_r
    }

    fn mark(&mut self, r: usize, need: usize, end: f64) {
        for i in r..(r + need).min(self.end_times.len()) {
            self.end_times[i] = end;
        }
    }
}

/// コメント列から ASS 文字列を生成する。
///
/// コメントは `vpos_ms` 昇順に処理される (レーン割り当ては時間順前提)。
pub fn generate_ass(comments: &[BurnInComment], opts: &AssOptions) -> String {
    let width = opts.width.max(1) as f64;
    let height = opts.height.max(1) as f64;
    let lane_height = font_px(Size::Medium, opts).max(1.0);
    let num_lanes = ((height / lane_height).floor() as usize).max(1);

    let mut scroll = ScrollLanes::new(num_lanes, width, opts.scroll_duration_sec.max(0.1));
    let mut ue = FixedLanes::new(num_lanes);
    let mut shita = FixedLanes::new(num_lanes);

    // vpos 昇順に並べ替えてから処理 (呼び出し側がソート済みでも安全側に)。
    let mut ordered: Vec<&BurnInComment> = comments.iter().collect();
    ordered.sort_by_key(|c| c.vpos_ms);

    let style_alpha = alpha_hex(opts.opacity);
    let outline_px = (lane_height * 0.04).max(1.0);

    let mut events = String::new();

    for c in ordered {
        if c.content.trim().is_empty() {
            continue;
        }
        let style = parse_style(&c.commands, c.is_owner);
        if style.invisible {
            continue;
        }
        let start = c.vpos_ms as f64 / 1000.0;
        // 動画長を超えて出現するコメントは描かない (duration_sec<=0 なら制限なし)。
        if opts.duration_sec > 0.0 && start > opts.duration_sec {
            continue;
        }

        let fsize = font_px(style.size, opts);
        let lines: Vec<&str> = c.content.split('\n').collect();
        let line_count = lines.len().max(1);
        let max_line_w = lines
            .iter()
            .map(|l| estimate_line_width(l, fsize))
            .fold(0.0_f64, f64::max)
            .max(1.0);
        // このコメントが占める高さ → 必要レーン数。
        let block_h = fsize * line_count as f64;
        let need = ((block_h / lane_height).ceil() as usize).max(1);

        let color_tag = ass_color(style.rgb);
        let outline_tag = ass_color(outline_color_for(style.rgb));
        let fs_int = fsize.round().max(1.0) as i64;
        let text = ass_escape(&c.content);

        let (tags, end) = match style.position {
            Position::Naka => {
                let lane = scroll.allocate(start, max_line_w, need);
                let y_top = lane as f64 * lane_height;
                let end = start + opts.scroll_duration_sec;
                // \an7 = 左上基準。左端を画面右外 (width) → 左外 (-幅) へ移動。
                let x_start = width;
                let x_end = -max_line_w;
                let tags = format!(
                    "{{\\an7\\fs{fs_int}\\c{color_tag}\\3c{outline_tag}\
                     \\move({x1},{y},{x2},{y})}}",
                    x1 = x_start.round() as i64,
                    x2 = x_end.round() as i64,
                    y = y_top.round() as i64,
                );
                (tags, end)
            }
            Position::Ue => {
                let lane = ue.allocate(start, start + opts.fixed_duration_sec, need);
                let y_top = lane as f64 * lane_height;
                let end = start + opts.fixed_duration_sec;
                // \an8 = 上中央基準。libass が水平センタリングしてくれる。
                let tags = format!(
                    "{{\\an8\\fs{fs_int}\\c{color_tag}\\3c{outline_tag}\\pos({x},{y})}}",
                    x = (width / 2.0).round() as i64,
                    y = y_top.round() as i64,
                );
                (tags, end)
            }
            Position::Shita => {
                let lane = shita.allocate(start, start + opts.fixed_duration_sec, need);
                // 下詰め: lane 0 = 最下段。ブロック上端を計算する。
                let y_top = height - (lane + need) as f64 * lane_height;
                let end = start + opts.fixed_duration_sec;
                let tags = format!(
                    "{{\\an8\\fs{fs_int}\\c{color_tag}\\3c{outline_tag}\\pos({x},{y})}}",
                    x = (width / 2.0).round() as i64,
                    y = y_top.round().max(0.0) as i64,
                );
                (tags, end)
            }
        };

        events.push_str(&format!(
            "Dialogue: 0,{},{},nnd,,0,0,0,,{}{}\n",
            fmt_time(start),
            fmt_time(end),
            tags,
            text,
        ));
    }

    let header = format!(
        "[Script Info]\n\
         ; Generated by Re:NNDD comment burn-in\n\
         ScriptType: v4.00+\n\
         PlayResX: {w}\n\
         PlayResY: {h}\n\
         WrapStyle: 2\n\
         ScaledBorderAndShadow: yes\n\
         YCbCr Matrix: TV.601\n\
         \n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
         BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
         BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: nnd,{font},{fs},&H{alpha}FFFFFF,&H{alpha}FFFFFF,&H{alpha}000000,&H80000000,\
         1,0,0,0,100,100,0,0,1,{outline:.1},0,7,0,0,0,1\n\
         \n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        w = opts.width.max(1),
        h = opts.height.max(1),
        font = opts.font_name,
        fs = lane_height.round().max(1.0) as i64,
        alpha = style_alpha,
        outline = outline_px,
    );

    format!("{header}{events}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn opts_1080() -> AssOptions {
        AssOptions {
            width: 1920,
            height: 1080,
            duration_sec: 600.0,
            ..AssOptions::default()
        }
    }

    fn cmt(vpos_ms: i64, content: &str, commands: &[&str]) -> BurnInComment {
        BurnInComment {
            vpos_ms,
            content: content.to_string(),
            commands: commands.iter().map(|s| s.to_string()).collect(),
            is_owner: false,
        }
    }

    // ---- 色 ----

    #[test]
    fn named_colors_resolve() {
        assert_eq!(named_color("red"), Some(0xFF0000));
        assert_eq!(named_color("white"), Some(0xFFFFFF));
        assert_eq!(named_color("niconicowhite"), Some(0xCCCC99));
        assert_eq!(named_color("truered"), Some(0xCC0033));
        assert_eq!(named_color("marineblue"), Some(0x3399FF));
        assert_eq!(named_color("notacolor"), None);
    }

    #[test]
    fn hex_colors_parse() {
        assert_eq!(parse_hex_color("#FF8800"), Some(0xFF8800));
        assert_eq!(parse_hex_color("#f80"), Some(0xFF8800));
        assert_eq!(parse_hex_color("FF8800"), None); // # 必須
        assert_eq!(parse_hex_color("#xyz"), None);
    }

    #[test]
    fn ass_color_is_bgr() {
        // 0xRRGGBB=FF0000 (赤) → &H0000FF&
        assert_eq!(ass_color(0xFF0000), "&H0000FF&");
        // 緑
        assert_eq!(ass_color(0x00FF00), "&H00FF00&");
        // 青
        assert_eq!(ass_color(0x0000FF), "&HFF0000&");
    }

    // ---- コマンド解釈 ----

    #[test]
    fn parse_defaults_to_naka_medium_white() {
        let s = parse_style(&[], false);
        assert_eq!(s.position, Position::Naka);
        assert_eq!(s.size, Size::Medium);
        assert_eq!(s.rgb, 0xFFFFFF);
        assert!(!s.invisible);
    }

    #[test]
    fn parse_position_size_color() {
        let s = parse_style(&["shita".into(), "big".into(), "red".into()], false);
        assert_eq!(s.position, Position::Shita);
        assert_eq!(s.size, Size::Big);
        assert_eq!(s.rgb, 0xFF0000);
    }

    #[test]
    fn parse_last_color_wins_and_hex() {
        let s = parse_style(&["red".into(), "#00FF00".into()], false);
        assert_eq!(s.rgb, 0x00FF00);
    }

    #[test]
    fn parse_is_case_insensitive() {
        let s = parse_style(&["UE".into(), "BIG".into(), "Red".into()], false);
        assert_eq!(s.position, Position::Ue);
        assert_eq!(s.size, Size::Big);
        assert_eq!(s.rgb, 0xFF0000);
    }

    #[test]
    fn parse_invisible_flag() {
        let s = parse_style(&["invisible".into()], false);
        assert!(s.invisible);
    }

    // ---- エスケープ ----

    #[test]
    fn escape_braces_and_backslash() {
        assert_eq!(ass_escape("a{b}c"), "a\\{b\\}c");
        assert_eq!(ass_escape("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_newline_to_hard_break() {
        assert_eq!(ass_escape("line1\nline2"), "line1\\Nline2");
    }

    #[test]
    fn escape_leading_space_to_hardspace() {
        assert_eq!(ass_escape("  hi"), "\\h\\hhi");
    }

    // ---- 時刻 ----

    #[test]
    fn time_formatting() {
        assert_eq!(fmt_time(0.0), "0:00:00.00");
        assert_eq!(fmt_time(1.5), "0:00:01.50");
        assert_eq!(fmt_time(61.23), "0:01:01.23");
        assert_eq!(fmt_time(3661.0), "1:01:01.00");
    }

    // ---- アルファ ----

    #[test]
    fn alpha_mapping() {
        assert_eq!(alpha_hex(1.0), "00");
        assert_eq!(alpha_hex(0.0), "FF");
        assert_eq!(alpha_hex(0.5), "80");
    }

    // ---- 生成全体 ----

    #[test]
    fn header_has_playres_and_style() {
        let ass = generate_ass(&[], &opts_1080());
        assert!(ass.contains("PlayResX: 1920"));
        assert!(ass.contains("PlayResY: 1080"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("Style: nnd,"));
        assert!(ass.contains("[Events]"));
    }

    #[test]
    fn empty_comments_have_no_dialogue() {
        let ass = generate_ass(&[], &opts_1080());
        assert!(!ass.contains("Dialogue:"));
    }

    #[test]
    fn naka_comment_uses_move() {
        let ass = generate_ass(&[cmt(1000, "流れるコメント", &[])], &opts_1080());
        assert!(ass.contains("Dialogue: 0,0:00:01.00,"));
        assert!(ass.contains("\\move("));
        assert!(ass.contains("流れるコメント"));
    }

    #[test]
    fn ue_comment_is_fixed_top() {
        let ass = generate_ass(&[cmt(2000, "上コメ", &["ue"])], &opts_1080());
        assert!(ass.contains("\\an8"));
        assert!(ass.contains("\\pos("));
        // 上固定は 3 秒表示。
        assert!(ass.contains("0:00:02.00,0:00:05.00"));
    }

    #[test]
    fn shita_comment_is_near_bottom() {
        let ass = generate_ass(&[cmt(0, "下コメ", &["shita"])], &opts_1080());
        // 下端付近の y が出るはず (lane_height≈74 → y_top ≈ 1080-74=1006 前後)。
        // \pos(960,<y>) の y が height の半分より大きい (下半分)。
        let line = ass.lines().find(|l| l.starts_with("Dialogue:")).unwrap();
        let y: i64 = line
            .split("\\pos(960,")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .and_then(|s| s.parse().ok())
            .unwrap();
        assert!(y > 540, "shita comment should sit in lower half, got y={y}");
    }

    #[test]
    fn invisible_comment_skipped() {
        let ass = generate_ass(&[cmt(1000, "見えない", &["invisible"])], &opts_1080());
        assert!(!ass.contains("Dialogue:"));
    }

    #[test]
    fn blank_comment_skipped() {
        let ass = generate_ass(&[cmt(1000, "   ", &[])], &opts_1080());
        assert!(!ass.contains("Dialogue:"));
    }

    #[test]
    fn comment_after_duration_skipped() {
        let mut o = opts_1080();
        o.duration_sec = 5.0;
        let ass = generate_ass(&[cmt(10_000, "遅刻", &[])], &o);
        assert!(!ass.contains("Dialogue:"));
    }

    #[test]
    fn color_command_emits_color_tag() {
        let ass = generate_ass(&[cmt(0, "赤", &["red"])], &opts_1080());
        // 赤 0xFF0000 → &H0000FF&
        assert!(ass.contains("\\c&H0000FF&"));
    }

    #[test]
    fn simultaneous_naka_comments_use_different_lanes() {
        // 同時刻に 2 つ流すと別の y (レーン) に載るはず。
        let ass = generate_ass(
            &[cmt(0, "あいうえお", &[]), cmt(0, "かきくけこ", &[])],
            &opts_1080(),
        );
        let ys: Vec<i64> = ass
            .lines()
            .filter(|l| l.starts_with("Dialogue:"))
            .filter_map(|l| {
                // \move(x1,y,x2,y) の最初の y を拾う。
                let after = l.split("\\move(").nth(1)?;
                let mut it = after.split(',');
                let _x1 = it.next()?;
                it.next()?.parse::<i64>().ok()
            })
            .collect();
        assert_eq!(ys.len(), 2);
        assert_ne!(ys[0], ys[1], "overlapping comments must not share a lane");
    }

    #[test]
    fn sequential_naka_comments_can_reuse_lane() {
        // 1 つめが流れ切ってから 2 つめが出れば同じ y (レーン 0) を再利用できる。
        let ass = generate_ass(&[cmt(0, "短", &[]), cmt(8000, "次", &[])], &opts_1080());
        let ys: Vec<i64> = ass
            .lines()
            .filter(|l| l.starts_with("Dialogue:"))
            .filter_map(|l| {
                let after = l.split("\\move(").nth(1)?;
                let mut it = after.split(',');
                let _x1 = it.next()?;
                it.next()?.parse::<i64>().ok()
            })
            .collect();
        assert_eq!(ys, vec![0, 0]);
    }

    #[test]
    fn comments_sorted_by_vpos() {
        // 入力が逆順でも時刻昇順で出力される。
        let ass = generate_ass(&[cmt(5000, "後", &[]), cmt(1000, "先", &[])], &opts_1080());
        let first = ass.lines().find(|l| l.starts_with("Dialogue:")).unwrap();
        assert!(first.contains("先"));
        assert!(first.contains("0:00:01.00"));
    }

    #[test]
    fn opacity_bakes_into_style_alpha() {
        let mut o = opts_1080();
        o.opacity = 0.5;
        let ass = generate_ass(&[], &o);
        assert!(ass.contains("&H80FFFFFF"));
    }

    #[test]
    fn font_name_used_in_style() {
        let mut o = opts_1080();
        o.font_name = "Noto Sans CJK JP".into();
        let ass = generate_ass(&[], &o);
        assert!(ass.contains("Style: nnd,Noto Sans CJK JP,"));
    }

    #[test]
    fn braces_in_content_are_escaped_not_tags() {
        let ass = generate_ass(&[cmt(0, "{evil}", &[])], &opts_1080());
        // 本文の { } はエスケープされ、override ブロックとして解釈されない。
        assert!(ass.contains("\\{evil\\}"));
    }
}

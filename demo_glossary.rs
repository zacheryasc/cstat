// rustc demo_glossary.rs -o demo_glossary && ./demo_glossary
// Full glossary of terminal rendering primitives

fn main() {
    println!("\n{}", "═".repeat(70));
    println!("  TERMINAL RENDERING GLOSSARY — COMPLETE CATALOG");
    println!("{}\n", "═".repeat(70));

    section_pixel_blocks();
    section_line_drawing();
    section_color_and_style();
    section_palettes();
    section_chart_primitives();
    section_text_symbols();
    section_layout_patterns();
    section_terminal_features();
    section_distribution_viz();
    section_comparison_viz();
    section_relational_viz();
    section_matrix_grid_viz();
    section_part_to_whole_viz();
    section_temporal_viz();
    section_text_integrated_viz();
}

// ── Helpers ──────────────────────────────────────────────────────────

fn fg(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, text)
}
fn bg(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[48;2;{};{};{}m{}\x1b[0m", r, g, b, text)
}
fn fgbg(fr: u8, fgg: u8, fb: u8, br: u8, bgg: u8, bb: u8, text: &str) -> String {
    format!("\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m{}\x1b[0m", fr, fgg, fb, br, bgg, bb, text)
}
fn lerp(c1: (u8,u8,u8), c2: (u8,u8,u8), t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (c1.0 as f64 + (c2.0 as f64 - c1.0 as f64) * t) as u8,
        (c1.1 as f64 + (c2.1 as f64 - c1.1 as f64) * t) as u8,
        (c1.2 as f64 + (c2.2 as f64 - c1.2 as f64) * t) as u8,
    )
}
fn heat(t: f64) -> (u8,u8,u8) {
    if t < 0.5 { lerp((80,200,120),(240,200,60),t*2.0) }
    else { lerp((240,200,60),(220,60,60),(t-0.5)*2.0) }
}
fn viridis(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(68,1,84)),(0.25,(59,82,139)),(0.5,(33,145,140)),(0.75,(94,201,98)),(1.0,(253,231,37))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn magma(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(0,0,4)),(0.25,(81,18,124)),(0.5,(183,55,121)),(0.75,(252,137,97)),(1.0,(252,253,191))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn inferno(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(0,0,4)),(0.25,(87,16,110)),(0.5,(188,55,84)),(0.75,(249,142,9)),(1.0,(252,255,164))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn plasma(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(13,8,135)),(0.25,(126,3,168)),(0.5,(204,71,120)),(0.75,(248,149,64)),(1.0,(240,249,33))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn cividis(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(0,32,77)),(0.25,(60,77,110)),(0.5,(127,127,127)),(0.75,(186,173,107)),(1.0,(255,234,70))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn coolwarm(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(59,76,192)),(0.25,(124,159,230)),(0.5,(221,221,221)),(0.75,(230,145,113)),(1.0,(180,4,38))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[4].1
}
fn turbo(t: f64) -> (u8,u8,u8) {
    let t = t.clamp(0.0, 1.0);
    let s = [(0.0,(48,18,59)),(0.17,(69,117,180)),(0.33,(64,190,166)),(0.5,(145,224,79)),(0.67,(241,187,41)),(0.83,(237,105,37)),(1.0,(122,4,3))];
    for i in 0..s.len()-1 { if t <= s[i+1].0 { return lerp(s[i].1,s[i+1].1,(t-s[i].0)/(s[i+1].0-s[i].0)); } }
    s[6].1
}

fn heading(id: &str, title: &str) {
    println!("\n\x1b[1;96m╔{}╗\x1b[0m", "═".repeat(68));
    println!("\x1b[1;96m║\x1b[0m  \x1b[1;97m{}\x1b[0m. \x1b[1;93m{:<62}\x1b[0m\x1b[1;96m║\x1b[0m", id, title);
    println!("\x1b[1;96m╚{}╝\x1b[0m\n", "═".repeat(68));
}

fn sub(id: &str, name: &str, desc: &str) {
    println!("  \x1b[1;33m{}.\x1b[0m \x1b[1m{}\x1b[0m", id, name);
    println!("     \x1b[2m{}\x1b[0m", desc);
}

// =====================================================================
// A. PIXEL/BLOCK ELEMENTS
// =====================================================================

fn section_pixel_blocks() {
    heading("A", "PIXEL / BLOCK ELEMENTS");

    // A1: Horizontal fractional blocks
    sub("A1", "Horizontal Fractional Blocks", "Sub-character bar precision (8 levels per cell)");
    println!("     chars: █ ▉ ▊ ▋ ▌ ▍ ▎ ▏");
    print!("     demo:  ");
    let fracs = ['█','▉','▊','▋','▌','▍','▎','▏'];
    for (i, &c) in fracs.iter().enumerate() {
        let t = i as f64 / 7.0;
        let (r,g,b) = heat(1.0 - t);
        print!("{}", fg(r,g,b, &format!("{} ", c)));
    }
    println!();
    print!("     bar:   ");
    for i in 0..30 {
        let t = i as f64 / 29.0;
        let (r,g,b) = viridis(t);
        print!("{}", fg(r,g,b,"█"));
    }
    print!("{}",fg(94,201,98,"▌"));
    println!("  ← fractional end cap");
    println!();

    // A2: Vertical block elements (sparkline chars)
    sub("A2", "Vertical Block Elements", "Height encoding (8 levels per cell, used in sparklines)");
    println!("     chars: ▁ ▂ ▃ ▄ ▅ ▆ ▇ █");
    let vblocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    print!("     demo:  ");
    let vals = [1,3,5,8,6,4,7,8,5,3,2,4,6,8,7,5,3,2,1,3,5,7,8,6,4,2,1,2,4,6];
    for &v in &vals {
        let t = v as f64 / 8.0;
        let (r,g,b) = heat(t);
        print!("{}", fg(r,g,b, &vblocks[(v-1) as usize].to_string()));
    }
    println!();
    println!();

    // A3: Half blocks
    sub("A3", "Half Blocks (▀ ▄)", "2 vertical pixels per cell — doubles vertical resolution");
    println!("     chars: ▀ (upper) ▄ (lower) █ (both)");
    print!("     demo:  ");
    // Show a mini gradient with 2 rows packed per line
    for i in 0..30 {
        let t_top = i as f64 / 29.0;
        let t_bot = (i as f64 + 0.5) / 29.0;
        let top = viridis(t_top);
        let bot = viridis(t_bot.min(1.0));
        print!("{}", fgbg(top.0,top.1,top.2, bot.0,bot.1,bot.2, "▀"));
    }
    println!("  ← 2 color rows in 1 line");
    // Show as mini heatmap
    print!("     matrix: ");
    let cells = [[0.1,0.3,0.8,0.9],[0.2,0.5,0.7,0.4],[0.6,0.2,0.1,0.3],[0.9,0.8,0.4,0.2]];
    for pair in cells.chunks(2) {
        for col in 0..4 {
            let top = viridis(pair[0][col]);
            let bot = if pair.len() > 1 { viridis(pair[1][col]) } else { (0,0,0) };
            print!("{}", fgbg(top.0,top.1,top.2, bot.0,bot.1,bot.2, "▀▀"));
        }
        print!("  ");
    }
    println!("← 4×4 matrix in 2 lines");
    println!();

    // A4: Quadrant blocks
    sub("A4", "Quadrant Blocks", "2×2 pixel grid per cell (4 sub-pixels)");
    println!("     chars: ▖ ▗ ▘ ▙ ▚ ▛ ▜ ▝ ▌ ▐ ▀ ▄ █ (space)");
    let quads = [' ','▘','▝','▀','▖','▌','▞','▛','▗','▚','▐','▜','▄','▙','▟','█'];
    print!("     all:   ");
    for (i, &q) in quads.iter().enumerate() {
        let t = i as f64 / 15.0;
        let (r,g,b) = viridis(t);
        print!("{}", fg(r,g,b, &format!("{} ", q)));
    }
    println!();
    print!("     pattern:");
    let pattern = "▘▀▜█▛▀▘ ▖▄▟█▙▄▖ ";
    for ch in pattern.chars() {
        print!("{}", fg(100,180,255, &ch.to_string()));
    }
    println!();
    println!();

    // A5: Braille patterns
    sub("A5", "Braille Patterns", "2×4 pixel grid per cell (8 sub-pixels, 256 combinations)");
    println!("     base: U+2800, bits: ⠁⠂⠄⡀ (left col) ⠈⠐⠠⢀ (right col)");
    print!("     gradient: ");
    let braille_densities: [u8; 9] = [0x00, 0x40, 0x44, 0x64, 0x66, 0x76, 0x77, 0xF7, 0xFF];
    for &b in &braille_densities {
        let ch = char::from_u32(0x2800 + b as u32).unwrap();
        print!("{} ", fg(100,180,255, &ch.to_string()));
    }
    println!(" (empty → full)");
    // Mini scatter
    print!("     scatter: ");
    let mut seed: u64 = 42;
    let mut canvas = [[0u8; 25]; 6]; // 6 rows × 25 cols of braille cells
    for _ in 0..120 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = ((seed >> 33) as f64 / u32::MAX as f64 * 50.0) as usize; // pixel x
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let raw_y = (seed >> 33) as f64 / u32::MAX as f64;
        let y = (raw_y * raw_y * 24.0) as usize; // pixel y, clustered low
        let cx = x / 2; let cy = y / 4;
        if cx < 25 && cy < 6 {
            let lx = x % 2; let ly = y % 4;
            let bit = match (lx, ly) { (0,0)=>0,(0,1)=>1,(0,2)=>2,(0,3)=>6,(1,0)=>3,(1,1)=>4,(1,2)=>5,(1,3)=>7,_=>0 };
            canvas[cy][cx] |= 1 << bit;
        }
    }
    for row in 0..6 {
        if row > 0 { print!("              "); }
        for col in 0..25 {
            let ch = char::from_u32(0x2800 + canvas[row][col] as u32).unwrap();
            if canvas[row][col] == 0 { print!("\x1b[2m·\x1b[0m"); }
            else { print!("{}", fg(80,200,120, &ch.to_string())); }
        }
        println!();
    }
    println!();

    // A6: Shade blocks
    sub("A6", "Shade/Fill Blocks", "4 density levels for area fills and backgrounds");
    println!("     chars: ░ (light 25%) ▒ (medium 50%) ▓ (dark 75%) █ (full)");
    print!("     demo:  ");
    let shades = ['░','▒','▓','█'];
    for &s in &shades {
        let t = match s { '░'=>0.25,'▒'=>0.5,'▓'=>0.75,_=>1.0 };
        let (r,g,b) = heat(t);
        print!("{}", fg(r,g,b, &format!("{}{}{} ", s, s, s)));
    }
    println!();
    // Show as background fill
    print!("     fill:  ");
    print!("{}",bg(40,40,60,"  empty  "));
    print!("{}",bg(60,60,80," ░░░25%░░░ "));
    print!("{}",bg(80,80,100," ▒▒50%▒▒ "));
    print!("{}",bg(100,100,120," ▓▓75%▓▓ "));
    print!("{}",bg(120,120,140," ██100%██ "));
    println!();
    println!();

    // A7: Block sextants (Unicode 13.0)
    sub("A7", "Block Sextants (Unicode 13.0+)", "2×3 pixel grid per cell (64 combinations). Newer — check terminal support.");
    println!("     chars: 🬀🬁🬂🬃🬄🬅🬆🬇🬈🬉🬊🬋🬌🬍🬎🬏🬐🬑🬒🬓🬔🬕🬖🬗🬘🬙🬚🬛🬜🬝🬞🬟🬠🬡🬢🬣🬤🬥🬦🬧🬨🬩🬪🬫🬬🬭🬮🬯🬰🬱🬲🬳🬴🬵🬶🬷🬸🬹🬺🬻");
    print!("     sample: ");
    let sextants = ['🬀','🬁','🬃','🬇','🬏','🬟','🬯','🬻'];
    for &s in &sextants {
        print!("{} ", fg(100,180,255, &s.to_string()));
    }
    println!(" (empty → full, may not render in all terminals)");
    println!();

    // A8: Left/Right half blocks
    sub("A8", "Left/Right Half Blocks", "2 horizontal pixels per cell");
    println!("     chars: ▌ (left half) ▐ (right half)");
    print!("     demo:  ");
    for i in 0..20 {
        let t = i as f64 / 19.0;
        let left = viridis(t);
        let right = viridis((t + 0.025).min(1.0));
        print!("{}", fgbg(left.0,left.1,left.2, right.0,right.1,right.2, "▌"));
    }
    println!("  ← 40 color columns in 20 chars");
    println!();
}

// =====================================================================
// B. LINE DRAWING
// =====================================================================

fn section_line_drawing() {
    heading("B", "LINE / BOX DRAWING");

    sub("B1", "Thin Lines", "Standard box-drawing (current cstat style)");
    println!("     ┌───┬───┐");
    println!("     │   │   │");
    println!("     ├───┼───┤");
    println!("     │   │   │");
    println!("     └───┴───┘");
    println!();

    sub("B2", "Rounded Corners", "Softer look, same weight");
    println!("     ╭───┬───╮");
    println!("     │   │   │");
    println!("     ├───┼───┤");
    println!("     │   │   │");
    println!("     ╰───┴───╯");
    println!();

    sub("B3", "Heavy/Thick Lines", "For emphasis or outer borders");
    println!("     ┏━━━┳━━━┓");
    println!("     ┃   ┃   ┃");
    println!("     ┣━━━╋━━━┫");
    println!("     ┃   ┃   ┃");
    println!("     ┗━━━┻━━━┛");
    println!();

    sub("B4", "Double Lines", "For major section boundaries");
    println!("     ╔═══╦═══╗");
    println!("     ║   ║   ║");
    println!("     ╠═══╬═══╣");
    println!("     ║   ║   ║");
    println!("     ╚═══╩═══╝");
    println!();

    sub("B5", "Mixed Weight", "Heavy outer, thin inner — visual hierarchy");
    println!("     ┏━━━┯━━━┓");
    println!("     ┃   │   ┃");
    println!("     ┠───┼───┨");
    println!("     ┃   │   ┃");
    println!("     ┗━━━┷━━━┛");
    println!();

    sub("B6", "Dashed/Dotted Lines", "For optional/weak connections");
    println!("     ╌╌╌ dashed thin     ┄┄┄ dotted thin");
    println!("     ╍╍╍ dashed heavy    ┅┅┅ dotted heavy");
    println!("     ┆ dashed thin vert  ┇ dashed heavy vert");
    println!("     ┈┈┈ more dotted     ┉┉┉ more dashed heavy");
    println!();

    sub("B7", "Diagonal Lines", "For crossings, X marks, slashes");
    println!("     chars: ╱ ╲ ╳");
    println!("     ╱╲╱╲╱╲   ╳╳╳╳");
    println!("     ╲╱╲╱╲╱   ╳╳╳╳");
    println!();

    sub("B8", "Arc Corners", "For connecting curved paths");
    println!("     ╭─╮    ╭──────╮");
    println!("     │ │    │ text │");
    println!("     │ ╰────╯      │");
    println!("     ╰─────────────╯");
    println!();
}

// =====================================================================
// C. COLOR AND STYLE
// =====================================================================

fn section_color_and_style() {
    heading("C", "COLOR & STYLE MODES");

    sub("C1", "Basic 8-Color", "Maximum compatibility (current cstat approach)");
    print!("     ");
    for (name, code) in [("black",30),("red",31),("green",32),("yellow",33),("blue",34),("magenta",35),("cyan",36),("white",37)] {
        print!("\x1b[{}m{}\x1b[0m ", code, name);
    }
    println!();
    println!();

    sub("C2", "Bright/Bold 8-Color", "16 colors total with bright variants");
    print!("     ");
    for code in 90..=97 {
        print!("\x1b[{}m████\x1b[0m", code);
    }
    println!();
    println!();

    sub("C3", "256-Color (8-bit)", "Wider palette, good compatibility");
    print!("     ");
    for i in (16..232).step_by(6) {
        print!("\x1b[38;5;{}m█\x1b[0m", i);
    }
    println!();
    print!("     ");
    for i in 232..=255 {
        print!("\x1b[38;5;{}m█\x1b[0m", i);
    }
    println!("  ← grayscale ramp");
    println!();

    sub("C4", "True Color (24-bit)", "16.7 million colors, smooth gradients");
    print!("     ");
    for i in 0..60 {
        let t = i as f64 / 59.0;
        let r = (t * 255.0) as u8;
        let g = ((1.0 - (t - 0.5).abs() * 2.0).max(0.0) * 255.0) as u8;
        let b = ((1.0 - t) * 255.0) as u8;
        print!("{}", fg(r, g, b, "█"));
    }
    println!();
    println!();

    sub("C5", "Text Styles", "Decorations available via ANSI SGR codes");
    println!("     \x1b[1mbold\x1b[0m  \x1b[2mdim\x1b[0m  \x1b[3mitalic\x1b[0m  \x1b[4munderline\x1b[0m  \x1b[9mstrikethrough\x1b[0m  \x1b[7mreverse\x1b[0m  \x1b[53moverline\x1b[0m");
    println!();

    sub("C6", "Colored Underlines", "Underline with independent color (modern terminals)");
    println!("     \x1b[4m\x1b[58;2;255;80;80mred underline\x1b[0m  \x1b[4m\x1b[58;2;80;255;80mgreen underline\x1b[0m  \x1b[4m\x1b[58;2;80;80;255mblue underline\x1b[0m");
    println!("     \x1b[4:3m\x1b[58;2;255;180;0mcurly/wavy underline\x1b[0m  \x1b[4:4m\x1b[58;2;100;200;255mdotted underline\x1b[0m  \x1b[4:5m\x1b[58;2;200;100;255mdashed underline\x1b[0m");
    println!();

    sub("C7", "Combined Foreground + Background", "Text on colored backgrounds for cells/badges");
    print!("     ");
    print!("{} ", fgbg(255,255,255, 220,60,60, " CRITICAL "));
    print!("{} ", fgbg(0,0,0, 240,200,60, " WARNING "));
    print!("{} ", fgbg(255,255,255, 80,200,120, "  OK  "));
    print!("{} ", fgbg(200,200,200, 60,60,80, " INFO "));
    println!();
    println!();
}

// =====================================================================
// D. COLOR PALETTES
// =====================================================================

fn section_palettes() {
    heading("D", "COLOR PALETTES");

    let palettes: Vec<(&str, fn(f64)->(u8,u8,u8), &str)> = vec![
        ("heat",    heat,     "red←bad  good→green (current intent, but smooth)"),
        ("viridis", viridis,  "perceptually uniform, colorblind-safe"),
        ("magma",   magma,    "dark→hot, high contrast on dark backgrounds"),
        ("inferno", inferno,  "similar to magma, more yellow"),
        ("plasma",  plasma,   "purple→yellow, vivid"),
        ("cividis", cividis,  "blue→yellow, fully colorblind-safe"),
        ("coolwarm",coolwarm, "diverging: blue=low, neutral=mid, red=high"),
        ("turbo",   turbo,    "rainbow-like, high contrast (not perceptually uniform)"),
    ];

    for (i, (name, pal, desc)) in palettes.iter().enumerate() {
        print!("  \x1b[1;33mD{}.\x1b[0m {:<10}", i+1, name);
        for j in 0..50 {
            let t = j as f64 / 49.0;
            let (r,g,b) = pal(t);
            print!("{}", fg(r,g,b, "█"));
        }
        println!();
        println!("               \x1b[2m{}\x1b[0m", desc);
    }

    println!("\n  \x1b[1;33mD9.\x1b[0m \x1b[1mCustom semantic\x1b[0m");
    println!("               \x1b[2mMap meaning to color, not just position\x1b[0m");
    print!("               ");
    // Semantic: green=safe, yellow=caution, red=danger, with smooth blending
    let semantic = [(0.0,(40,160,80)),(0.3,(40,160,80)),(0.5,(220,200,40)),(0.7,(220,200,40)),(0.85,(200,60,60)),(1.0,(200,60,60))];
    for j in 0..50 {
        let t = j as f64 / 49.0;
        let mut c = semantic[0].1;
        for k in 0..semantic.len()-1 {
            if t >= semantic[k].0 && t <= semantic[k+1].0 {
                let local = (t - semantic[k].0) / (semantic[k+1].0 - semantic[k].0);
                c = lerp(semantic[k].1, semantic[k+1].1, local);
                break;
            }
        }
        print!("{}", fg(c.0,c.1,c.2, "█"));
    }
    println!("  safe──caution──danger");
    println!();
}

// =====================================================================
// E. CHART PRIMITIVES
// =====================================================================

fn section_chart_primitives() {
    heading("E", "CHART / VISUALIZATION PRIMITIVES");

    // E1: Horizontal bar variants
    sub("E1", "Horizontal Bar Variants", "Different fill styles for bars");
    let _w = 30.0_f64;
    let fills: [(&str, &str); 6] = [
        ("solid",     "█"),
        ("shade-grad",""),   // special
        ("dotted",    "⣿"),
        ("hash",      "▓"),
        ("half",      "▌"),
        ("pipe",      "┃"),
    ];
    for (name, ch) in &fills {
        print!("     {:<12}", name);
        if *name == "shade-grad" {
            let shades = ['░','▒','▓','█'];
            for i in 0..30 {
                let t = i as f64 / 29.0;
                let idx = (t * 3.0).round() as usize;
                let (r,g,b) = heat(t);
                print!("{}", fg(r,g,b, &shades[idx.min(3)].to_string()));
            }
        } else {
            for i in 0..30 {
                let t = i as f64 / 29.0;
                let (r,g,b) = viridis(t);
                print!("{}", fg(r,g,b, ch));
            }
        }
        println!();
    }
    println!();

    // E2: Stacked bars
    sub("E2", "Stacked Bars", "Multiple values in one bar row");
    let stacks = [
        ("module_a", vec![(12, (80,200,120)), (8, (100,180,255)), (3, (240,200,60))]),
        ("module_b", vec![(20, (80,200,120)), (5, (100,180,255)), (1, (240,200,60))]),
        ("module_c", vec![(6,  (80,200,120)), (15,(100,180,255)), (10,(240,200,60))]),
    ];
    for (name, segments) in &stacks {
        print!("     {:<12}", name);
        for (len, (r,g,b)) in segments {
            print!("{}", fg(*r,*g,*b, &"█".repeat(*len)));
        }
        println!();
    }
    print!("     {:<12}", "");
    print!("{} code  ", fg(80,200,120, "██"));
    print!("{} tests  ", fg(100,180,255, "██"));
    print!("{} docs", fg(240,200,60, "██"));
    println!();
    println!();

    // E3: Grouped bars
    sub("E3", "Grouped Bars", "Side-by-side comparison per category");
    let groups = [("v1.0", 15), ("v1.1", 22), ("v2.0", 18)];
    let colors = [(220,80,80),(80,200,120),(100,180,255)];
    for (i, (label, val)) in groups.iter().enumerate() {
        let (r,g,b) = colors[i];
        println!("     {:<6} {} {}", label, fg(r,g,b, &"█".repeat(*val)), val);
    }
    println!();

    // E4: Vertical bars (column chart)
    sub("E4", "Vertical Column Charts", "Bottom-up using vertical block chars");
    let col_vals = [3,7,5,8,4,6,2,7,5,3,6,8,4,5,7,3];
    let max_h = 8;
    for row in (1..=max_h).rev() {
        print!("     ");
        for &v in &col_vals {
            if v >= row {
                let t = v as f64 / max_h as f64;
                let (r,g,b) = heat(t);
                print!("{}", fg(r,g,b, "██"));
            } else if v == row - 1 {
                // fractional top
                let t = v as f64 / max_h as f64;
                let (r,g,b) = heat(t);
                print!("{}", fg(r,g,b, "▄ "));
            } else {
                print!("  ");
            }
        }
        println!();
    }
    print!("     ");
    for _ in &col_vals { print!("──"); }
    println!();
    println!();

    // E5: Gauge/meter
    sub("E5", "Gauge / Progress Meter", "Bounded bar with track");
    let gauges = [("Health", 0.78), ("Coverage", 0.45), ("Coupling", 0.92)];
    for (label, val) in &gauges {
        let filled = (*val * 25.0) as usize;
        let empty = 25 - filled;
        let (r,g,b) = heat(*val);
        let bar = format!("{}{}",
            fg(r,g,b, &"█".repeat(filled)),
            "\x1b[2m░\x1b[0m".repeat(empty));
        println!("     {:<12} [{}] {:>5.1}%", label, bar, val * 100.0);
    }
    println!();

    // E6: Waffle/grid chart
    sub("E6", "Waffle Chart", "Grid of filled/empty squares showing proportion");
    let total = 100;
    let filled_count = 73;
    print!("     73/100: ");
    for i in 0..total {
        if i > 0 && i % 25 == 0 { print!(" "); }
        if i < filled_count {
            let t = i as f64 / total as f64;
            let (r,g,b) = viridis(t);
            print!("{}", fg(r,g,b, "■"));
        } else {
            print!("\x1b[2m□\x1b[0m");
        }
    }
    println!();
    println!();

    // E7: Dot matrix / LED digits
    sub("E7", "Large Numeral Display", "Big digits for hero metrics using braille/blocks");
    // Simple 3x5 font
    let digits_3x5: [&[&str]; 10] = [
        &["▄█▄","█ █","█ █","█ █","▀█▀"], // 0
        &[" █ "," █ "," █ "," █ "," █ "], // 1
        &["▄█▄","  █","▄█▄","█  ","▀█▀"], // 2
        &["▄█▄","  █","▄█▄","  █","▀█▀"], // 3
        &["█ █","█ █","▀█▀","  █","  █"], // 4
        &["▀█▀","█  ","▀█▀","  █","▄█▄"], // 5
        &["▄█▄","█  ","██▄","█ █","▀█▀"], // 6
        &["▀█▀","  █","  █","  █","  █"], // 7
        &["▄█▄","█ █","▄█▄","█ █","▀█▀"], // 8
        &["▄█▄","█ █","▀██","  █","▀█▀"], // 9
    ];
    let number = [7, 8]; // display "78"
    for row in 0..5 {
        print!("     ");
        for &d in &number {
            print!("{} ", fg(253,231,37, digits_3x5[d][row]));
        }
        println!();
    }
    println!("     \x1b[2m(hero score display)\x1b[0m");
    println!();

    // E8: Flame chart style
    sub("E8", "Flame / Waterfall Bars", "Nested indented colored bars showing hierarchy/depth");
    let flames = [
        (0, "main()", 40),
        (1, "analyze()", 35),
        (2, "parse_all()", 20),
        (3, "parse_file()", 15),
        (2, "compute_metrics()", 12),
        (1, "render()", 5),
    ];
    for (depth, name, width) in &flames {
        let indent = "  ".repeat(*depth);
        let t = *depth as f64 / 3.0;
        let (r,g,b) = inferno(0.3 + t * 0.5);
        print!("     {}", indent);
        for _ in 0..*width { print!("{}", fg(r,g,b, "▓")); }
        println!(" {}", name);
    }
    println!();

    // E9: Trend arrows / indicators
    sub("E9", "Trend Indicators", "Compact directional symbols for changes");
    println!("     ↑ ↗ → ↘ ↓    (arrows)");
    println!("     ▲ △ ▶ ▽ ▼    (triangles)");
    println!("     {} {} {} {} {}",
        fg(220,60,60, "▲+15%"),
        fg(220,60,60, "↑ 8%"),
        fg(150,150,150, "→ 0%"),
        fg(80,200,120, "↓ 3%"),
        fg(80,200,120, "▼-12%"),
    );
    println!();

    // E10: Area under sparkline
    sub("E10", "Filled Sparklines", "Sparkline with area fill underneath");
    let spark_vals = [2,4,3,6,8,7,5,8,6,4,3,5,7,8,6,4,2,3,5,7,6,4,3,2,4,6,7,5,3,2];
    let max_v = 8;
    let blocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    print!("     line:   ");
    for &v in &spark_vals {
        let t = v as f64 / max_v as f64;
        let (r,g,b) = viridis(t);
        print!("{}", fg(r,g,b, &blocks[((v-1) as usize).min(7)].to_string()));
    }
    println!();
    print!("     filled: ");
    for &v in &spark_vals {
        let t = v as f64 / max_v as f64;
        let top = viridis(t);
        let bot = viridis(t * 0.5);
        print!("{}", fgbg(top.0,top.1,top.2, bot.0,bot.1,bot.2, &blocks[((v-1) as usize).min(7)].to_string()));
    }
    println!();
    println!();

    // E11: Box plot
    sub("E11", "Inline Box Plot", "Min, Q1, median, Q3, max in one line");
    // ├──────┤    ╞══════╡    │   ┣━━━╋━━━┫
    print!("     ");
    print!("\x1b[2m├──────\x1b[0m");
    print!("{}", fg(100,180,255, "┤█████████"));
    print!("{}", fg(253,231,37, "│"));
    print!("{}", fg(100,180,255, "██████████┤"));
    print!("\x1b[2m──────────┤\x1b[0m");
    println!();
    println!("     \x1b[2mmin    Q1       med         Q3          max\x1b[0m");
    println!();

    // E12: Dot strip / strip plot
    sub("E12", "Dot/Strip Plot", "Individual data points on a number line");
    print!("     \x1b[2m0\x1b[0m");
    let dots = [3,5,5,6,8,8,8,12,14,15,15,16,18,22,25,30];
    let max_d = 35;
    let mut line = vec![' '; max_d + 1];
    for &d in &dots {
        line[d] = '●';
    }
    for (i, &ch) in line.iter().enumerate() {
        if ch == '●' {
            let t = i as f64 / max_d as f64;
            let (r,g,b) = heat(t);
            print!("{}", fg(r,g,b, "●"));
        } else {
            print!("\x1b[2m·\x1b[0m");
        }
    }
    println!(" \x1b[2m{}\x1b[0m", max_d);
    println!();
}

// =====================================================================
// F. TEXT & SYMBOL ELEMENTS
// =====================================================================

fn section_text_symbols() {
    heading("F", "TEXT & SYMBOL ELEMENTS");

    sub("F1", "Status Indicators", "Semantic symbols for pass/fail/warn states");
    println!("     {} pass   {} fail   {} warn   {} info   {} skip   {} pending",
        fg(80,200,120, "✓"),
        fg(220,60,60, "✗"),
        fg(240,200,60, "⚠"),
        fg(100,180,255, "ℹ"),
        fg(150,150,150, "⊘"),
        fg(200,200,200, "◌"),
    );
    println!("     {} {} {} {} {} {}",
        fg(80,200,120, "●"),
        fg(220,60,60, "●"),
        fg(240,200,60, "●"),
        fg(100,180,255, "●"),
        fg(150,150,150, "○"),
        fg(200,200,200, "◐"),
    );
    println!();

    sub("F2", "Bullets & List Markers", "For ranked lists, trees, enumerations");
    println!("     • ◦ ‣ ⁃ ▸ ▹ ▪ ▫ ◆ ◇ ◈ ❖");
    println!();

    sub("F3", "Arrows & Connectors", "For flow, relationships, direction");
    println!("     → ← ↑ ↓ ↔ ↕ ↗ ↘ ↙ ↖");
    println!("     ⟶ ⟵ ⟷ ⇒ ⇐ ⇔ ⇨ ⇦");
    println!("     ↳ ↱ ↰ ↲  (turns)");
    println!("     ➜ ➤ ▶ ◀  (filled)");
    println!();

    sub("F4", "Superscript & Subscript Numbers", "For footnotes, exponents, indices");
    println!("     super: ⁰ ¹ ² ³ ⁴ ⁵ ⁶ ⁷ ⁸ ⁹ ⁺ ⁻ ⁼ ⁽ ⁾ ⁿ");
    println!("     sub:   ₀ ₁ ₂ ₃ ₄ ₅ ₆ ₇ ₈ ₉ ₊ ₋ ₌ ₍ ₎");
    println!("     usage: O(n²)  σ₃=1.2  f⁽ⁿ⁾");
    println!();

    sub("F5", "Mathematical Symbols", "For formulas, stats, annotations");
    println!("     μ σ Σ Π ∫ ∂ ∇ √ ∞ ≈ ≠ ≤ ≥ ± ÷ × ∈ ∉ ⊂ ⊃ ∪ ∩ ∅ ∀ ∃");
    println!("     usage: μ=12.3  σ=4.1  n=312  Σ=3847");
    println!();

    sub("F6", "Stars & Ratings", "For scores, quality ratings");
    print!("     ");
    for i in 0..5 {
        if i < 3 { print!("{}", fg(253,231,37, "★")); }
        else { print!("{}", fg(80,80,80, "☆")); }
    }
    println!("  3/5 stars");
    print!("     ");
    // Fractional rating with half star
    for i in 0..5 {
        if i < 3 { print!("{}", fg(253,231,37, "★")); }
        else if i == 3 { print!("{}",fg(253,231,37,"⯪")); }
        else { print!("{}", fg(80,80,80, "☆")); }
    }
    println!("  3.5/5 stars");
    println!();

    sub("F7", "Enclosed/Circled Characters", "For labels, badges, numbered references");
    println!("     numbers: ① ② ③ ④ ⑤ ⑥ ⑦ ⑧ ⑨ ⑩");
    println!("     filled:  ❶ ❷ ❸ ❹ ❺ ❻ ❼ ❽ ❾ ❿");
    println!("     letters: Ⓐ Ⓑ Ⓒ Ⓓ Ⓔ  ⓐ ⓑ ⓒ ⓓ ⓔ");
    println!();

    sub("F8", "Dice & Cards", "Fun alternative for small integer values");
    println!("     dice: ⚀ ⚁ ⚂ ⚃ ⚄ ⚅  (1-6)");
    println!();

    sub("F9", "Separators & Ornaments", "Section dividers beyond plain lines");
    println!("     ─ ═ ━ ╌ ╍ ┄ ┅ ┈ ┉");
    println!("     ·····  ‧‧‧‧‧  ⋯⋯⋯⋯⋯  …………");
    print!("     ");
    for i in 0..50 {
        let t = i as f64 / 49.0;
        let (r,g,b) = viridis(t);
        print!("{}", fg(r,g,b, "─"));
    }
    println!();
    print!("     ");
    for i in 0..50 {
        let t = i as f64 / 49.0;
        let (r,g,b) = magma(t);
        if i % 2 == 0 { print!("{}", fg(r,g,b, "═")); }
        else { print!("{}", fg(r,g,b, " ")); }
    }
    println!();
    println!();
}

// =====================================================================
// G. LAYOUT PATTERNS
// =====================================================================

fn section_layout_patterns() {
    heading("G", "LAYOUT PATTERNS");

    sub("G1", "Side-by-Side Panels", "Two data views sharing one row of terminal lines");
    let left = vec![
        "╭──── Stats ────╮",
        "│ Files:     47 │",
        "│ Funcs:    312 │",
        "│ LoC:   12,847 │",
        "╰────────────────╯",
    ];
    let right = vec![
        "╭──── Health ───╮",
        "│ Score:     78% │",
        "│ Trend:      ↗  │",
        "│ Grade:      B+ │",
        "╰────────────────╯",
    ];
    for i in 0..left.len() {
        println!("     {}   {}", left[i], right.get(i).unwrap_or(&""));
    }
    println!();

    sub("G2", "Tree / Indent View", "Hierarchical data with connecting lines");
    let tree = [
        ("src/",          0, false),
        ("├── main.rs",   1, false),
        ("├── render.rs",  1, false),
        ("├── analysis/",  1, false),
        ("│   ├── loc.rs",    2, false),
        ("│   ├── complexity.rs", 2, false),
        ("│   └── deps/",     2, false),
        ("│       ├── mod.rs",   3, false),
        ("│       └── render.rs",3, true),
        ("└── summary/",   1, false),
        ("    ├── mod.rs",    2, false),
        ("    └── sections.rs",2, true),
    ];
    for (line, depth, _is_last) in &tree {
        let t = *depth as f64 / 3.0;
        let (r,g,b) = viridis(t * 0.8 + 0.2);
        println!("     {}", fg(r,g,b, line));
    }
    println!();

    sub("G3", "Tab-Style Headers", "Section navigation indicators");
    print!("     ");
    print!("{}", fgbg(0,0,0, 100,180,255, " Summary "));
    print!("{}", fgbg(200,200,200, 40,40,50, " LoC "));
    print!("{}", fgbg(200,200,200, 40,40,50, " Complexity "));
    print!("{}", fgbg(200,200,200, 40,40,50, " Deps "));
    print!("{}", fgbg(200,200,200, 40,40,50, " Graph "));
    println!();
    print!("     ");
    print!("{}", fg(100,180,255, "━━━━━━━━━"));
    print!("{}", fg(60,60,70,    "╸────────────╺────────────╸──────╺───────╸"));
    println!();
    println!();

    sub("G4", "Inline Key-Value with Separators", "Compact horizontal metadata");
    print!("     ");
    print!("{}", fg(100,180,255, "47"));
    print!(" files \x1b[2m│\x1b[0m ");
    print!("{}", fg(100,180,255, "312"));
    print!(" functions \x1b[2m│\x1b[0m ");
    print!("{}", fg(100,180,255, "12,847"));
    print!(" LoC \x1b[2m│\x1b[0m ");
    print!("complexity μ=");
    print!("{}", fg(240,200,60, "8.3"));
    println!();
    println!();

    sub("G5", "Badge / Pill Labels", "Highlighted inline labels");
    print!("     ");
    print!("{}", fgbg(255,255,255, 180,60,60,  " critical "));
    print!(" ");
    print!("{}", fgbg(0,0,0, 240,200,60,  " moderate "));
    print!(" ");
    print!("{}", fgbg(255,255,255, 60,160,80,  " healthy "));
    print!(" ");
    print!("{}", fgbg(200,200,200, 60,60,80,  " neutral "));
    print!(" ");
    print!("{}", fgbg(255,255,255, 100,100,180, " info "));
    println!();
    println!();

    sub("G6", "Nested Boxes", "Boxes within boxes for grouped data");
    println!("     ╭─── Module: render ────────────────────╮");
    println!("     │  ╭── Functions ──────╮  ╭── Stats ──╮ │");
    println!("     │  │ terminal_width()  │  │ LoC:  174 │ │");
    println!("     │  │ visible_len()     │  │ CC:   3.2 │ │");
    println!("     │  │ bar_color()       │  │ Deps:   4 │ │");
    println!("     │  ╰──────────────────╯  ╰──────────╯ │");
    println!("     ╰───────────────────────────────────────╯");
    println!();

    sub("G7", "Responsive Column Width", "Adapt layout to terminal width (already partially done)");
    println!("     \x1b[2mNarrow (<60):  single column, truncated labels\x1b[0m");
    println!("     \x1b[2mMedium (60-100): standard layout\x1b[0m");
    println!("     \x1b[2mWide (>100): side-by-side panels, expanded labels\x1b[0m");
    println!();
}

// =====================================================================
// H. TERMINAL FEATURES
// =====================================================================

fn section_terminal_features() {
    heading("H", "TERMINAL-LEVEL FEATURES");

    sub("H1", "Clickable Hyperlinks (OSC 8)", "Terminal links to files/URLs — click to open");
    println!("     \x1b]8;;file:///workspace/src/render.rs\x1b\\src/render.rs:42\x1b]8;;\x1b\\  ← try clicking (supported: iTerm2, WezTerm, Windows Terminal, Kitty, etc.)");
    println!("     \x1b]8;;https://example.com\x1b\\https://example.com\x1b]8;;\x1b\\");
    println!();

    sub("H2", "In-Place Updates (Cursor Movement)", "Overwrite previous output for live-feel");
    println!("     \\x1b[nA = move up n lines    \\x1b[nB = move down");
    println!("     \\x1b[nC = move right n cols  \\x1b[nD = move left");
    println!("     \\x1b[2K = clear current line  \\r = carriage return");
    println!("     \x1b[2mUseful for: progress bars, status updates, animation\x1b[0m");
    println!();

    sub("H3", "Terminal Title (OSC 2)", "Set the terminal window/tab title");
    println!("     \\x1b]2;cstat — analyzing project\\x07");
    println!("     \x1b[2mSets window title to reflect current operation\x1b[0m");
    println!();

    sub("H4", "Alternate Screen Buffer", "Switch to full-screen mode and back");
    println!("     \\x1b[?1049h = enter alternate screen");
    println!("     \\x1b[?1049l = leave alternate screen");
    println!("     \x1b[2mUseful for: a full-screen dashboard mode without losing scroll history\x1b[0m");
    println!();

    sub("H5", "Sixel Graphics", "Actual raster images in terminal (limited support)");
    println!("     \x1b[2mSupported: xterm, mlterm, WezTerm, foot, some others\x1b[0m");
    println!("     \x1b[2mCould render: actual scatter plots, treemaps, architecture diagrams as images\x1b[0m");
    println!();

    sub("H6", "Kitty Graphics Protocol", "High-quality image display (Kitty terminal)");
    println!("     \x1b[2mSupported: Kitty, WezTerm\x1b[0m");
    println!("     \x1b[2mCould render: PNG charts inline in output\x1b[0m");
    println!();

    sub("H7", "Notification (OSC 9/777)", "Send desktop notification when analysis completes");
    println!("     \\x1b]9;Analysis complete\\x07  (Windows Terminal)");
    println!("     \\x1b]777;notify;cstat;Done\\x07  (rxvt-unicode)");
    println!();

    println!();
}

// =====================================================================
// I. DISTRIBUTION VISUALIZATIONS
// =====================================================================

fn section_distribution_viz() {
    heading("I", "DISTRIBUTION VISUALIZATIONS");

    // I1: Violin plot
    sub("I1", "Violin Plot", "Mirrored density curve — shows distribution shape, not just quartiles");
    // Simulate a density: peaked in the middle, long right tail
    let density = [1,2,3,5,8,12,15,18,15,11,8,6,5,4,3,3,2,2,1,1];
    let max_d = 18;
    let h = density.len();
    println!("     \x1b[2m         cyclomatic           cognitive\x1b[0m");
    let density2 = [1,1,2,4,7,10,14,10,7,5,4,3,2,2,1,1,1,0,0,0];
    for row in 0..h {
        let d1 = density[row];
        let d2 = density2[row];
        let w1 = (d1 as f64 / max_d as f64 * 15.0).round() as usize;
        let w2 = (d2 as f64 / max_d as f64 * 15.0).round() as usize;
        let (r1,g1,b1) = viridis(d1 as f64 / max_d as f64);
        let (r2,g2,b2) = magma(d2 as f64 / max_d as f64);
        let left_pad = 15 - w1;
        print!("     {:>3}\x1b[2m│\x1b[0m", if row == 0 || row == h-1 || row == h/2 { format!("{}", row) } else { String::new() });
        print!("{}{}", " ".repeat(left_pad), fg(r1,g1,b1, &"█".repeat(w1)));
        print!("\x1b[2m│\x1b[0m");
        print!("{}", fg(r1,g1,b1, &"█".repeat(w1)));
        print!("{}  ", " ".repeat(left_pad));
        // second violin
        let left_pad2 = 15 - w2;
        print!("{}{}", " ".repeat(left_pad2), fg(r2,g2,b2, &"█".repeat(w2)));
        print!("\x1b[2m│\x1b[0m");
        print!("{}", fg(r2,g2,b2, &"█".repeat(w2)));
        println!();
    }
    println!();

    // I2: Ridgeline / joy plot
    sub("I2", "Ridgeline / Joy Plot", "Overlapping distributions — compare shapes across groups");
    let blocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    let ridges: [(&str, [u8; 20]); 4] = [
        ("render ",  [0,1,2,4,6,7,8,7,5,3,2,1,1,0,0,0,0,0,0,0]),
        ("deps   ",  [0,0,1,2,3,5,7,8,7,6,5,4,3,2,1,1,0,0,0,0]),
        ("complex",  [0,0,0,1,1,2,3,4,5,7,8,7,6,4,3,2,1,1,0,0]),
        ("loc    ",  [0,0,0,0,0,1,1,2,3,4,5,6,7,8,8,7,5,3,1,0]),
    ];
    for (name, vals) in &ridges {
        print!("     {}\x1b[2m│\x1b[0m ", name);
        for &v in vals {
            if v == 0 {
                print!(" ");
            } else {
                let t = v as f64 / 8.0;
                let (r,g,b) = viridis(t);
                print!("{}", fg(r,g,b, &blocks[(v as usize - 1).min(7)].to_string()));
            }
        }
        println!();
    }
    println!("     \x1b[2m       └──────────────────────\x1b[0m");
    println!();

    // I3: Beeswarm plot
    sub("I3", "Beeswarm / Jitter Plot", "Individual points jittered to avoid overlap — shows density + outliers");
    let swarm_data: [(f64, &[i8]); 1] = [(0.0, &[])]; // placeholder, we'll hardcode the visual
    let _ = swarm_data;
    // Render a beeswarm: each column is a value bucket, dots stack vertically
    println!("     \x1b[2m         5    10   15   20   25   30   35\x1b[0m");
    // Row by row, dots placed at various x positions with jitter
    let rows: [&str; 5] = [
        "              ●              ●         ",
        "         ●    ●●   ●    ●              ",
        "     ●   ●●   ●●●  ●●   ●   ●    ●   ",
        "    ●●   ●●●  ●●●● ●●●  ●●  ●●   ●   ",
        "   ●●●●  ●●●● ●●●● ●●●● ●●● ●●●  ●●  ",
    ];
    for (i, row) in rows.iter().enumerate() {
        let t = 1.0 - i as f64 / 4.0;
        print!("     ");
        for ch in row.chars() {
            if ch == '●' {
                let (r,g,b) = viridis(0.3 + t * 0.5);
                print!("{}", fg(r,g,b, "●"));
            } else {
                print!(" ");
            }
        }
        println!();
    }
    println!("     \x1b[2m    └─────────────────────────────────────\x1b[0m");
    println!();

    // I4: CDF / Cumulative Distribution
    sub("I4", "CDF / Cumulative Distribution Curve", "What % of values fall below threshold X — uses braille for smooth curve");
    let cdf_vals = [0.0,0.02,0.05,0.10,0.18,0.28,0.40,0.52,0.63,0.73,0.80,0.86,0.90,0.93,0.95,0.97,0.98,0.99,0.99,1.0];
    let cdf_h = 8;
    let cdf_w = cdf_vals.len();
    for row in (0..cdf_h).rev() {
        let threshold = row as f64 / (cdf_h - 1) as f64;
        print!("     {:>4.0}%\x1b[2m│\x1b[0m", threshold * 100.0);
        for (col, &v) in cdf_vals.iter().enumerate() {
            if (v - threshold).abs() < 0.08 {
                let (r,g,b) = viridis(v);
                print!("{}", fg(r,g,b, "●━"));
            } else if v > threshold {
                print!("  ");
            } else {
                print!("  ");
            }
        }
        println!();
    }
    println!("     \x1b[2m    └──────────────────────────────────────\x1b[0m");
    println!("     \x1b[2m    0          complexity →             max\x1b[0m");
    println!();

    // I5: Rug plot
    sub("I5", "Rug Plot", "Tick marks along axis edge — shows each individual observation");
    print!("     \x1b[2m│\x1b[0m");
    let rug_points = [2,3,5,5,6,7,7,7,8,10,11,11,12,14,15,18,22,25,28,35];
    let rug_max = 40;
    let mut rug_line = vec![' '; rug_max + 1];
    for &p in &rug_points {
        if p <= rug_max { rug_line[p] = '│'; }
    }
    for (i, &ch) in rug_line.iter().enumerate() {
        if ch == '│' {
            let t = i as f64 / rug_max as f64;
            let (r,g,b) = heat(t);
            print!("{}", fg(r,g,b, "│"));
        } else {
            print!("\x1b[2m╌\x1b[0m");
        }
    }
    println!("\x1b[2m│\x1b[0m");
    println!("     \x1b[2mEach tick = one observation. Dense ticks = concentration.\x1b[0m");
    println!();

    // I6: Q-Q Plot
    sub("I6", "Q-Q Plot", "Quantile-quantile — compare distribution to normal. Points on diagonal = normal.");
    let qq_h = 10;
    let qq_w = 30;
    // Points roughly along diagonal with some deviation
    let qq_points: [(usize,usize);12] = [(1,0),(3,2),(6,4),(9,6),(11,8),(13,10),(15,13),(18,16),(21,19),(23,22),(26,25),(29,28)];
    for row in (0..qq_h).rev() {
        print!("     \x1b[2m│\x1b[0m");
        for col in 0..qq_w {
            // diagonal reference line
            let on_diag = (col as f64 / qq_w as f64 - row as f64 / qq_h as f64).abs() < 0.06;
            let is_point = qq_points.iter().any(|&(px,py)| {
                px == col && (py as f64 / (qq_h as f64) * qq_h as f64).round() as usize == row
            });
            if is_point {
                print!("{}", fg(80,200,120, "●"));
            } else if on_diag {
                print!("\x1b[2m╱\x1b[0m");
            } else {
                print!(" ");
            }
        }
        println!();
    }
    println!("     \x1b[2m└──────────────────────────────\x1b[0m");
    println!("     \x1b[2mtheoretical quantiles →  (deviation from line = non-normality)\x1b[0m");
    println!();
}

// =====================================================================
// J. COMPARISON & RANKING VISUALIZATIONS
// =====================================================================

fn section_comparison_viz() {
    heading("J", "COMPARISON & RANKING VISUALIZATIONS");

    // J1: Lollipop chart
    sub("J1", "Lollipop Chart", "Dot on a stick — cleaner than bars for sparse/ranked data");
    let items = [
        ("parse_expr",     42),
        ("resolve_imports",31),
        ("build_graph",    24),
        ("validate_ast",   18),
        ("emit_warning",    7),
    ];
    let max_v = 42;
    for (name, val) in &items {
        let w = (*val as f64 / max_v as f64 * 35.0) as usize;
        let t = *val as f64 / max_v as f64;
        let (r,g,b) = heat(t);
        print!("     {:<20}", name);
        print!("{}", fg(r,g,b, &"╌".repeat(w)));
        print!("{}", fg(r,g,b, "●"));
        println!(" {}", val);
    }
    println!();

    // J2: Dumbbell chart
    sub("J2", "Dumbbell Chart", "Two dots connected — shows range or before/after delta");
    let pairs = [
        ("render.rs",     8, 14),
        ("complexity.rs", 22, 18),
        ("dist.rs",       15, 28),
        ("loc.rs",        12, 12),
    ];
    let scale = 35;
    let max_val = 30;
    println!("     {:<18} \x1b[2m{:>15} v1   v2\x1b[0m", "", "");
    for (name, v1, v2) in &pairs {
        let p1 = (*v1 as f64 / max_val as f64 * scale as f64) as usize;
        let p2 = (*v2 as f64 / max_val as f64 * scale as f64) as usize;
        let (lo, hi) = if p1 < p2 { (p1, p2) } else { (p2, p1) };
        let improved = v2 < v1;
        print!("     {:<18}", name);
        for i in 0..=scale {
            if i == p1 {
                print!("{}", fg(150,150,200, "●"));
            } else if i == p2 {
                if improved { print!("{}", fg(80,200,120, "●")); }
                else { print!("{}", fg(220,60,60, "●")); }
            } else if i > lo && i < hi {
                print!("─");
            } else {
                print!("\x1b[2m·\x1b[0m");
            }
        }
        println!(" {:>2}→{}", v1, v2);
    }
    println!();

    // J3: Slope chart
    sub("J3", "Slope Chart", "Two columns connected by slope lines — shows rank change");
    let slopes = [
        ("parse_expr",    1, 2),
        ("build_graph",   2, 1),
        ("resolve",       3, 5),
        ("validate",      4, 3),
        ("emit_warn",     5, 4),
    ];
    let col_gap = 25;
    println!("     \x1b[2m  v1.0                       v2.0\x1b[0m");
    // Position items at their rank
    let h = 5;
    for rank in 1..=h {
        let left = slopes.iter().find(|s| s.1 == rank);
        let right = slopes.iter().find(|s| s.2 == rank);
        let left_name = left.map(|s| s.0).unwrap_or("");
        let right_name = right.map(|s| s.0).unwrap_or("");
        let going_up = left.map(|s| s.2 < s.1).unwrap_or(false);
        let going_down = left.map(|s| s.2 > s.1).unwrap_or(false);
        let slope_char = if going_up { "╱" } else if going_down { "╲" } else { "─" };
        let (r,g,b) = if going_up { (80,200,120) } else if going_down { (220,60,60) } else { (150,150,150) };
        print!("     {:<14}", left_name);
        print!("{}", fg(r,g,b, "●"));
        print!("{}", fg(r,g,b, &format!("─{}─",slope_char).repeat(3)));
        print!("{}", if right.is_some() { fg(100,180,255,"●") } else { " ".to_string() });
        println!(" {}", right_name);
    }
    println!();

    // J4: Bump chart
    sub("J4", "Bump Chart", "Rank trajectories over time — who moved up/down");
    let bump_data: [(&str, [usize; 5]); 4] = [
        ("render",   [1,1,2,3,3]),
        ("deps",     [2,3,3,2,1]),
        ("complex",  [3,2,1,1,2]),
        ("loc",      [4,4,4,4,4]),
    ];
    let colors = [(220,80,80),(80,200,120),(100,180,255),(240,200,60)];
    println!("     \x1b[2m         t1   t2   t3   t4   t5\x1b[0m");
    for rank in 1..=4 {
        print!("     #{:<2}", rank);
        for t in 0..5 {
            let who = bump_data.iter().position(|d| d.1[t] == rank);
            if let Some(idx) = who {
                let (r,g,b) = colors[idx];
                print!("  {}", fg(r,g,b, &format!("{:─<4}", bump_data[idx].0)));
            } else {
                print!("      ");
            }
        }
        println!();
    }
    println!();

    // J5: Cleveland dot plot
    sub("J5", "Cleveland Dot Plot", "Aligned dots on a grid — precise comparison without bar clutter");
    let dot_items = [
        ("parse_expr",      42.0),
        ("resolve_imports", 31.0),
        ("build_graph",     24.5),
        ("validate_ast",    18.2),
        ("emit_warning",     7.0),
    ];
    let max_v = 45.0;
    println!("     \x1b[2m{:>20} 0        10        20        30        40\x1b[0m", "");
    println!("     \x1b[2m{:>20} ┼─────────┼─────────┼─────────┼─────────┼\x1b[0m", "");
    for (name, val) in &dot_items {
        let pos = (*val / max_v * 45.0) as usize;
        print!("     {:>20} ", name);
        for i in 0..=45 {
            if i == pos {
                let t = *val / max_v;
                let (r,g,b) = heat(t);
                print!("{}", fg(r,g,b, "◆"));
            } else if i % 10 == 0 {
                print!("\x1b[2m┊\x1b[0m");
            } else {
                print!("\x1b[2m·\x1b[0m");
            }
        }
        println!(" {:.1}", val);
    }
    println!();

    // J6: Diverging bar chart
    sub("J6", "Diverging Bar Chart", "Bars extend left/right from center — shows positive/negative change");
    let changes = [
        ("render.rs",      6),
        ("complexity.rs", -4),
        ("dist.rs",       13),
        ("loc.rs",         0),
        ("deps/mod.rs",   -8),
    ];
    let max_abs = 15;
    let half_w = 15;
    for (name, delta) in &changes {
        print!("     {:<16}", name);
        let abs_d = (*delta as f64).abs();
        let bar_w = (abs_d / max_abs as f64 * half_w as f64) as usize;
        if *delta < 0 {
            let pad = half_w - bar_w;
            let (r,g,b) = (100,180,255);
            print!("{}{}", " ".repeat(pad), fg(r,g,b, &"◄".repeat(bar_w)));
            print!("\x1b[2m│\x1b[0m");
            print!("{}", " ".repeat(half_w));
        } else if *delta > 0 {
            print!("{}", " ".repeat(half_w));
            print!("\x1b[2m│\x1b[0m");
            let (r,g,b) = (220,120,60);
            print!("{}", fg(r,g,b, &"►".repeat(bar_w)));
            print!("{}", " ".repeat(half_w - bar_w));
        } else {
            print!("{}", " ".repeat(half_w));
            print!("\x1b[2m│\x1b[0m");
            print!("{}", " ".repeat(half_w));
        }
        println!(" {:>+3}", delta);
    }
    println!();

    // J7: Radar/Spider chart
    sub("J7", "Radar / Spider Chart", "Multivariate profile — one polygon per entity, using braille canvas");
    // Simplified: show as a small braille rendering
    let metrics_labels = ["CC", "LoC", "Nest", "Deps", "Params"];
    let profile = [0.8, 0.5, 0.3, 0.7, 0.4]; // normalized 0-1
    // Render as a 5-axis star in braille
    let cx = 20_f64;
    let cy = 16_f64;
    let radius = 14.0_f64;
    let n_axes = 5;
    let mut canvas = vec![vec![0u8; 22]; 9]; // braille cells
    let mut density = vec![vec![0u32; 22]; 9];
    let pi = std::f64::consts::PI;
    // Draw axes and polygon
    for i in 0..n_axes {
        let angle = pi / 2.0 + i as f64 * 2.0 * pi / n_axes as f64;
        // Axis line
        for step in 0..30 {
            let t = step as f64 / 29.0;
            let px = (cx + angle.cos() * radius * t) as usize;
            let py = (cy - angle.sin() * radius * t) as usize; // flip Y
            set_braille(&mut canvas, &mut density, px, py);
        }
        // Polygon vertex
        let r = profile[i];
        let vx = (cx + angle.cos() * radius * r) as usize;
        let vy = (cy - angle.sin() * radius * r) as usize;
        set_braille(&mut canvas, &mut density, vx, vy);
        // Connect to next vertex
        let next = (i + 1) % n_axes;
        let next_angle = pi / 2.0 + next as f64 * 2.0 * pi / n_axes as f64;
        let next_r = profile[next];
        let nvx = cx + next_angle.cos() * radius * next_r;
        let nvy = cy - next_angle.sin() * radius * next_r;
        for step in 0..20 {
            let t = step as f64 / 19.0;
            let lx = (vx as f64 + (nvx - vx as f64) * t) as usize;
            let ly = (vy as f64 + (nvy - vy as f64) * t) as usize;
            set_braille(&mut canvas, &mut density, lx, ly);
        }
    }
    for row in 0..9 {
        print!("     ");
        for col in 0..22 {
            let ch = char::from_u32(0x2800 + canvas[row][col] as u32).unwrap();
            if canvas[row][col] == 0 {
                print!(" ");
            } else if density[row][col] > 2 {
                print!("{}", fg(253,231,37, &ch.to_string()));
            } else {
                print!("{}", fg(80,200,120, &ch.to_string()));
            }
        }
        // Labels at approximate positions
        match row {
            0 => print!("  {}", metrics_labels[0]),
            2 => print!("  {}", metrics_labels[1]),
            5 => print!("  {}", metrics_labels[4]),
            8 => print!("  \x1b[2m(polygon = module profile)\x1b[0m"),
            _ => {}
        }
        println!();
    }
    println!();
}

fn set_braille(canvas: &mut [Vec<u8>], density: &mut [Vec<u32>], px: usize, py: usize) {
    let cx = px / 2;
    let cy = py / 4;
    if cx >= canvas[0].len() || cy >= canvas.len() { return; }
    let lx = px % 2;
    let ly = py % 4;
    let bit = match (lx, ly) { (0,0)=>0,(0,1)=>1,(0,2)=>2,(0,3)=>6,(1,0)=>3,(1,1)=>4,(1,2)=>5,(1,3)=>7,_=>0 };
    canvas[cy][cx] |= 1 << bit;
    density[cy][cx] += 1;
}

// =====================================================================
// K. RELATIONAL / FLOW VISUALIZATIONS
// =====================================================================

fn section_relational_viz() {
    heading("K", "RELATIONAL & FLOW VISUALIZATIONS");

    // K1: Node-edge graph
    sub("K1", "Node-Edge Graph Layout", "2D graph with boxes and connecting lines");
    println!("     ╭────────╮          ╭──────────╮");
    println!("     │ render │─────────→│ summary  │");
    println!("     ╰────┬───╯          ╰────┬─────╯");
    println!("          │                    │");
    println!("          ↓                    ↓");
    println!("     ╭─────────╮         ╭──────────╮");
    println!("     │  deps   │←───────→│  graph   │");
    println!("     ╰────┬────╯         ╰──────────╯");
    println!("          │");
    println!("          ↓");
    println!("     ╭──────────╮");
    println!("     │ analysis │");
    println!("     ╰──────────╯");
    println!();

    // K2: Sankey / flow diagram
    sub("K2", "Sankey / Alluvial Flow", "Width-proportional paths between stages — shows flow volume");
    let (r1,g1,b1) = (100,180,255);
    let (r2,g2,b2) = (80,200,120);
    let (r3,g3,b3) = (240,200,60);
    let (r4,g4,b4) = (220,100,100);
    println!("     \x1b[2mInput          Processing       Output\x1b[0m");
    println!("     {}━━━━━━━━━━━━━━━{}━━━━━━━━━━━{}",
        fg(r1,g1,b1,"parse ████"),fg(r1,g1,b1,"████████"),fg(r1,g1,b1,"████ render"));
    println!("     {}━━━━━━━━━━{}",
        fg(r2,g2,b2,"      ████"),fg(r2,g2,b2,"━━━━━━━━━━━━━━████ json"));
    println!("     {}━━━━━━━━━━━━━━━{}━━━━━━━━━━━{}",
        fg(r3,g3,b3,"walk  ██"),fg(r3,g3,b3,"██████"),fg(r3,g3,b3,"██ summary"));
    println!("     {}━━━━━━━━━━━━━━━{}",
        fg(r4,g4,b4,"      ██"),fg(r4,g4,b4,"━━━━━━━━━━━━━━██ guide"));
    println!("     \x1b[2m(bar width ∝ data volume through each path)\x1b[0m");
    println!();

    // K3: Chord-style connection list
    sub("K3", "Chord / Arc Diagram", "Connections between items on a line — uses arcs above/below");
    println!("     \x1b[2m(flat layout, arcs show connections; height ∝ strength)\x1b[0m");
    // Top arcs
    println!("     {}      {}",
        fg(100,180,255, "    ╭───────────────╮"),
        "");
    println!("     {}  {}",
        fg(80,200,120,  "  ╭────╮"),
        fg(220,100,100, "       ╭────────╮"));
    println!("     {}  {}",
        fg(240,200,60,  "╭─╮"),
        "");
    print!("     ");
    let nodes = ["render","deps","graph","loc","summary","complex"];
    for n in &nodes {
        print!("{} ", fg(253,231,37, &format!("[{}]", &n[..3])));
    }
    println!();
    // Bottom arcs
    println!("     {}",
        fg(200,100,200, "      ╰──────────────────────╯"));
    println!();

    // K4: Dependency arrows with indentation
    sub("K4", "Layered Dependency View", "Modules in tiers with directional arrows between layers");
    println!("     \x1b[2m── Tier 0 (entry) ──────────────────────────\x1b[0m");
    print!("     ");
    print!("{}", fgbg(200,200,200, 40,60,80, " main "));
    println!();
    println!("       │         │");
    println!("       ↓         ↓");
    println!("     \x1b[2m── Tier 1 (orchestration) ──────────────────\x1b[0m");
    print!("     ");
    print!("{} ", fgbg(200,200,200, 40,80,60, " summary "));
    print!("{}", fgbg(200,200,200, 40,80,60, " flow "));
    println!();
    println!("       │    ╲    │");
    println!("       ↓     ╲   ↓");
    println!("     \x1b[2m── Tier 2 (analysis) ───────────────────────\x1b[0m");
    print!("     ");
    print!("{} ", fgbg(200,200,200, 60,40,80, " deps "));
    print!("{} ", fgbg(200,200,200, 60,40,80, " complexity "));
    print!("{}", fgbg(200,200,200, 60,40,80, " loc "));
    println!();
    println!();

    // K5: Dendrogram
    sub("K5", "Dendrogram / Cluster Tree", "Hierarchical clustering — which modules are most similar");
    println!("     \x1b[2mdistance  0.0        0.5        1.0\x1b[0m");
    println!("     {}", fg(100,180,255, "render ─────────────┐"));
    println!("     {}                    {}", fg(100,180,255, "summary ────────────┤"), fg(80,200,120, ""));
    println!("     {}                    {}",
        fg(100,180,255, "                    ├──────────┐"),
        "");
    println!("     {}", fg(240,200,60, "deps ──────┐        │          │"));
    println!("     {}",  fg(240,200,60, "graph ─────┤────────┘          │"));
    println!("     {}", fg(240,200,60, "           │                   │"));
    println!("     {}", fg(220,100,100, "complexity ────────────────────┘"));
    println!();

    // K6: Adjacency list view
    sub("K6", "Adjacency List (Compact)", "Text-based graph: each node lists its connections");
    let adj = [
        ("render",     vec!["deps","summary","loc"]),
        ("deps",       vec!["graph","render"]),
        ("summary",    vec!["complexity","deps","loc","graph"]),
        ("complexity", vec![]),
        ("loc",        vec![]),
    ];
    for (node, edges) in &adj {
        let edge_str: Vec<String> = edges.iter().map(|e| fg(100,180,255, e)).collect();
        if edges.is_empty() {
            println!("     {} → \x1b[2m(leaf)\x1b[0m", fg(253,231,37, node));
        } else {
            println!("     {} → {}", fg(253,231,37, node), edge_str.join(", "));
        }
    }
    println!();
}

// =====================================================================
// L. MATRIX & GRID VISUALIZATIONS
// =====================================================================

fn section_matrix_grid_viz() {
    heading("L", "MATRIX & GRID VISUALIZATIONS");

    // L1: Calendar heatmap
    sub("L1", "Calendar Heatmap", "Grid of days colored by value — weeks as columns, days as rows");
    let days = ["Mon","Tue","Wed","Thu","Fri","Sat","Sun"];
    let mut seed: u64 = 99;
    println!("     \x1b[2m     W1   W2   W3   W4   W5   W6   W7   W8\x1b[0m");
    for d in 0..7 {
        print!("     \x1b[2m{}\x1b[0m ", days[d]);
        for _w in 0..8 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let v = (seed >> 33) as f64 / u32::MAX as f64;
            let (r,g,b) = viridis(v);
            print!("{}", bg(r,g,b, "    "));
            print!(" ");
        }
        println!();
    }
    println!("     \x1b[2m(commit activity, complexity changes, etc.)\x1b[0m");
    println!();

    // L2: Weighted adjacency matrix with blocks
    sub("L2", "Weighted Adjacency Matrix", "Color intensity shows connection strength, not just 0/1/2");
    let mods = ["rend","deps","summ","comp","loc ","grph"];
    let n = mods.len();
    print!("     {:>6}", "");
    for m in &mods { print!(" {}", m); }
    println!();
    let mut seed2: u64 = 77;
    for i in 0..n {
        print!("     {:>5} ", mods[i]);
        for j in 0..n {
            if i == j {
                print!("{}", bg(40,40,40, " ·· "));
                print!(" ");
            } else {
                seed2 = seed2.wrapping_mul(6364136223846793005).wrapping_add(1);
                let v = (seed2 >> 33) as f64 / u32::MAX as f64;
                let (r,g,b) = magma(v);
                let label = format!("{:.1}", v * 10.0);
                print!("{}", fgbg(if v > 0.5 {0} else {200}, if v > 0.5 {0} else {200}, if v > 0.5 {0} else {200}, r,g,b, &format!("{:>4}", label)));
                print!(" ");
            }
        }
        println!();
    }
    println!();

    // L3: Mosaic / Marimekko chart
    sub("L3", "Mosaic / Marimekko Chart", "Variable-width columns — both width and height encode data");
    let categories = [("src",50),("tests",25),("bench",15),("docs",10)];
    let sub_cats = [(0.6,(80,200,120)),(0.25,(100,180,255)),(0.15,(240,200,60))]; // code/test/doc proportions
    let total_w: usize = 60;
    println!("     \x1b[2mcolumn width ∝ total LoC, row height ∝ composition\x1b[0m");
    for (frac, (r,g,b)) in &sub_cats {
        print!("     ");
        for (name, pct) in &categories {
            let col_w = (*pct as f64 / 100.0 * total_w as f64) as usize;
            let fill_h = (*frac * 3.0_f64).round() as usize;
            let _ = fill_h;
            print!("{}", bg(*r,*g,*b, &" ".repeat(col_w)));
        }
        println!();
    }
    print!("     ");
    for (name, pct) in &categories {
        let col_w = (*pct as f64 / 100.0 * total_w as f64) as usize;
        let centered = format!("{:^w$}", name, w=col_w);
        print!("\x1b[2m{}\x1b[0m", centered);
    }
    println!();
    println!();

    // L4: Heatmap with annotations
    sub("L4", "Annotated Heatmap", "Color + text in each cell — value visible on colored background");
    let ann_labels = ["CC","Nest","Deps"];
    let ann_data = [[8.2,3.1,5.0],[2.4,7.8,1.2],[4.5,2.0,9.1]];
    print!("     {:>8}", "");
    for l in &ann_labels { print!(" {:>8}", l); }
    println!();
    for i in 0..3 {
        print!("     {:>8}", ann_labels[i]);
        for j in 0..3 {
            let v = ann_data[i][j];
            let t = v / 10.0;
            let (r,g,b) = magma(t);
            let txt_color = if t > 0.5 { (0,0,0) } else { (220,220,220) };
            print!(" {}", fgbg(txt_color.0,txt_color.1,txt_color.2, r,g,b, &format!("  {:>4.1}  ", v)));
        }
        println!();
    }
    println!();
}

// =====================================================================
// M. PART-TO-WHOLE VISUALIZATIONS
// =====================================================================

fn section_part_to_whole_viz() {
    heading("M", "PART-TO-WHOLE VISUALIZATIONS");

    // M1: Icicle chart
    sub("M1", "Icicle Chart", "Top-down nested rectangles — like a rectangular sunburst");
    let (r1,g1,b1) = (80,100,180);
    let (r2,g2,b2) = (80,160,120);
    let (r3,g3,b3) = (180,160,80);
    let (r4,g4,b4) = (180,80,80);
    // Level 0: full width
    println!("     {}", bg(60,60,80, &format!("{:^60}", "project (12,847 LoC)")));
    // Level 1: split
    println!("     {}{}{}{}",
        bg(r1,g1,b1, &format!("{:^30}", "src/ (8420)")),
        bg(r2,g2,b2, &format!("{:^15}", "tests/ (3200)")),
        bg(r3,g3,b3, &format!("{:^10}", "bench/")),
        bg(r4,g4,b4, &format!("{:^5}", "doc")),
    );
    // Level 2: src/ split further
    let s = lerp((80,100,180),(120,140,220),0.3);
    let s2 = lerp((80,100,180),(120,140,220),0.6);
    let s3 = lerp((80,100,180),(120,140,220),0.9);
    println!("     {}{}{}{}",
        bg(s.0,s.1,s.2, &format!("{:^14}", "analysis/")),
        bg(s2.0,s2.1,s2.2, &format!("{:^10}", "render/")),
        bg(s3.0,s3.1,s3.2, &format!("{:^6}", "cli")),
        " ".repeat(30),
    );
    println!();

    // M2: Proportional area (squares)
    sub("M2", "Proportional Area Squares", "Square size encodes magnitude — better area perception than bars");
    let items = [("parse",42),("resolve",31),("build",24),("validate",18),("emit",7)];
    for (name, val) in &items {
        let side = ((*val as f64).sqrt() * 1.5) as usize;
        let t = *val as f64 / 42.0;
        let (r,g,b) = heat(t);
        print!("     {:<10}", name);
        for _row in 0..1 {
            for _ in 0..side { print!("{}", fg(r,g,b, "██")); }
        }
        println!(" ({})", val);
    }
    println!("     \x1b[2m(area ∝ value, not just width)\x1b[0m");
    println!();

    // M3: Nested treemap
    sub("M3", "Treemap (Nested)", "Proportional nested rectangles — area ∝ value");
    println!("     ┌──────────────────────┬─────────────────┐");
    println!("     │                      │                 │");
    println!("     │    {}    │   {}  │",
        fg(100,180,255, "src/analysis"),
        fg(80,200,120, "src/render"));
    println!("     │     {}       │    {}    │",
        fg(100,180,255, "(4200 LoC)"),
        fg(80,200,120, "(2100)"));
    println!("     │                      │                 │");
    println!("     ├───────────┬──────────┼────────┬────────┤");
    println!("     │           │          │        │        │");
    println!("     │  {} │  {} │ {} │ {} │",
        fg(240,200,60, "tests"),
        fg(220,100,100, "bench"),
        fg(200,100,200, "cli"),
        fg(150,150,150, "doc"));
    println!("     │  {} │  {} │ {} │ {} │",
        fg(240,200,60, "(3200)"),
        fg(220,100,100, "(890)"),
        fg(200,100,200, "(400)"),
        fg(150,150,150, "(57)"));
    println!("     └───────────┴──────────┴────────┴────────┘");
    println!();

    // M4: Donut / ring chart (braille)
    sub("M4", "Donut / Ring Chart", "Circular proportion display using braille — for 2-4 segments");
    let segments: [(f64, (u8,u8,u8)); 3] = [
        (0.55, (100,180,255)),  // code
        (0.30, (80,200,120)),   // tests
        (0.15, (240,200,60)),   // docs
    ];
    let cx = 16.0_f64;
    let cy = 16.0_f64;
    let outer = 14.0_f64;
    let inner = 8.0_f64;
    let pi = std::f64::consts::PI;
    let mut canvas = vec![vec![0u8; 18]; 9];
    let mut colors = vec![vec![(0u8,0u8,0u8); 18]; 9];

    let mut angle_start = 0.0_f64;
    for (frac, color) in &segments {
        let angle_end = angle_start + frac * 2.0 * pi;
        // Fill arc
        let steps = 100;
        for s in 0..steps {
            let a = angle_start + (angle_end - angle_start) * s as f64 / steps as f64;
            for rd in 0..10 {
                let r = inner + (outer - inner) * rd as f64 / 9.0;
                let px = (cx + a.cos() * r) as usize;
                let py = (cy - a.sin() * r) as usize;
                let bcx = px / 2;
                let bcy = py / 4;
                if bcx < 18 && bcy < 9 {
                    let lx = px % 2;
                    let ly = py % 4;
                    let bit = match (lx, ly) { (0,0)=>0,(0,1)=>1,(0,2)=>2,(0,3)=>6,(1,0)=>3,(1,1)=>4,(1,2)=>5,(1,3)=>7,_=>0 };
                    canvas[bcy][bcx] |= 1 << bit;
                    colors[bcy][bcx] = *color;
                }
            }
        }
        angle_start = angle_end;
    }
    for row in 0..9 {
        print!("     ");
        for col in 0..18 {
            let ch = char::from_u32(0x2800 + canvas[row][col] as u32).unwrap();
            if canvas[row][col] == 0 {
                print!(" ");
            } else {
                let (r,g,b) = colors[row][col];
                print!("{}", fg(r,g,b, &ch.to_string()));
            }
        }
        match row {
            2 => print!("  {} 55% code", fg(100,180,255, "██")),
            4 => print!("  {} 30% tests", fg(80,200,120, "██")),
            6 => print!("  {} 15% docs", fg(240,200,60, "██")),
            _ => {}
        }
        println!();
    }
    println!();

    // M5: Stacked percentage bar
    sub("M5", "Stacked 100% Bar", "Horizontal bar always fills to 100% — shows proportions");
    let total_w = 50;
    let segments_pct = [("code",55,(80,200,120)),("test",30,(100,180,255)),("docs",10,(240,200,60)),("cfg",5,(180,100,200))];
    print!("     ");
    for (name, pct, (r,g,b)) in &segments_pct {
        let w = (*pct as f64 / 100.0 * total_w as f64).round() as usize;
        let label = if w > name.len() + 2 { format!("{:^w$}", format!("{} {}%", name, pct), w=w) } else { format!("{:^w$}", "", w=w) };
        print!("{}", fgbg(if *pct > 20 {255} else {200},if *pct > 20 {255} else {200},if *pct > 20 {255} else {200}, *r,*g,*b, &label));
    }
    println!();
    println!();

    // M6: Parliament / hemicycle (just described, complex to render)
    sub("M6", "Waffle Grid (Categorized)", "Grid squares colored by category — each square = 1%");
    let grid_cats = [('■',(80,200,120),55),('■',(100,180,255),30),('■',(240,200,60),10),('■',(180,100,200),5)];
    print!("     ");
    let mut count = 0;
    for (ch, (r,g,b), n) in &grid_cats {
        for _ in 0..*n {
            if count > 0 && count % 25 == 0 { print!("\n     "); }
            print!("{}", fg(*r,*g,*b, &ch.to_string()));
            count += 1;
        }
    }
    println!();
    print!("     ");
    for (_,  (r,g,b), _) in &grid_cats {
        print!("{}  ", fg(*r,*g,*b, "■■"));
    }
    println!();
    println!("     \x1b[2mcode  tests  docs   cfg\x1b[0m");
    println!();
}

// =====================================================================
// N. TEMPORAL & SEQUENTIAL VISUALIZATIONS
// =====================================================================

fn section_temporal_viz() {
    heading("N", "TEMPORAL & SEQUENTIAL VISUALIZATIONS");

    // N1: Sparkline band (multiple aligned)
    sub("N1", "Sparkline Band", "Multiple aligned sparklines for cross-metric comparison");
    let blocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    let bands: [(&str, [u8;20], fn(f64)->(u8,u8,u8)); 4] = [
        ("CC   ", [2,3,4,3,5,6,5,4,6,7,8,7,6,5,6,7,6,5,4,3], viridis),
        ("LoC  ", [3,3,4,4,5,5,6,6,7,7,7,7,8,8,8,7,7,6,6,5], magma),
        ("Deps ", [1,2,2,3,3,4,4,5,5,5,6,6,6,7,7,7,7,8,8,8], inferno),
        ("Nest ", [5,5,4,4,3,3,3,2,2,2,3,3,4,4,3,3,2,2,1,1], plasma),
    ];
    for (name, vals, pal) in &bands {
        print!("     {}", name);
        for &v in vals {
            let t = v as f64 / 8.0;
            let (r,g,b) = pal(t);
            print!("{}", fg(r,g,b, &blocks[(v as usize - 1).min(7)].to_string()));
        }
        println!();
    }
    println!("     \x1b[2m     ← older              newer →\x1b[0m");
    println!();

    // N2: Horizon chart
    sub("N2", "Horizon Chart", "Folded bands — encodes magnitude via color layers, saves vertical space");
    // Values 0-8, but we fold at 4: values 0-4 use light shade, 4-8 overlay dark
    let hz_vals = [1,2,3,4,5,6,7,8,7,6,5,4,3,2,1,0,1,2,4,6,8,6,4,2,0,1,3,5,7,5];
    let fold_at = 4;
    print!("     layer1: ");
    for &v in &hz_vals {
        let base = v.min(fold_at);
        let t = base as f64 / fold_at as f64;
        let (r,g,b) = lerp((30,30,50),(80,140,200), t);
        print!("{}", bg(r,g,b, &blocks[(base as usize).min(7).max(1) - 1].to_string()));
    }
    println!();
    print!("     layer2: ");
    for &v in &hz_vals {
        let over = if v > fold_at { v - fold_at } else { 0 };
        let base = v.min(fold_at);
        let base_t = base as f64 / fold_at as f64;
        let (br,bg_c,bb) = lerp((30,30,50),(80,140,200), base_t);
        if over > 0 {
            let over_t = over as f64 / fold_at as f64;
            let (fr,fgc,fb) = lerp((100,160,220),(220,240,255), over_t);
            print!("{}", fgbg(fr,fgc,fb, br,bg_c,bb, &blocks[(over as usize).min(7).max(1) - 1].to_string()));
        } else {
            print!("{}", bg(br,bg_c,bb, " "));
        }
    }
    println!();
    println!("     \x1b[2m(same data: layer1 = base, layer2 = base + overflow folded on top)\x1b[0m");
    println!();

    // N3: Gantt / timeline chart
    sub("N3", "Gantt / Timeline Chart", "Horizontal bars on a time axis — phases, durations, overlaps");
    let gantt = [
        ("parse",     0,  8,  (100,180,255)),
        ("analyze",   5, 15, (80,200,120)),
        ("deps",      8, 12,  (240,200,60)),
        ("complexity",10, 18, (220,100,100)),
        ("render",   16, 22, (200,100,200)),
    ];
    let max_t = 24;
    let scale = 48;
    println!("     \x1b[2m{:>14} 0        5       10       15       20\x1b[0m", "");
    println!("     \x1b[2m{:>14} ┼────────┼────────┼────────┼────────┼\x1b[0m", "");
    for (name, start, end, (r,g,b)) in &gantt {
        print!("     {:>14} ", name);
        let s = (*start as f64 / max_t as f64 * scale as f64) as usize;
        let e = (*end as f64 / max_t as f64 * scale as f64) as usize;
        for i in 0..scale {
            if i >= s && i < e {
                print!("{}", fg(*r,*g,*b, "█"));
            } else {
                print!("\x1b[2m·\x1b[0m");
            }
        }
        println!();
    }
    println!();

    // N4: Step chart
    sub("N4", "Step Chart", "Discrete level changes — like sparkline but shows exact transitions");
    let step_vals: [usize; 15] = [2,2,4,4,4,7,7,3,3,5,5,5,8,8,6];
    let max_s = 8;
    for row in (1..=max_s).rev() {
        print!("     {:>2}\x1b[2m│\x1b[0m", row);
        for (i, &v) in step_vals.iter().enumerate() {
            if v == row {
                let t = v as f64 / max_s as f64;
                let (r,g,b) = viridis(t);
                // horizontal segment
                print!("{}", fg(r,g,b, "──"));
                // vertical connector to next if different
            } else if i > 0 && step_vals[i-1] == row && v != row {
                // vertical going down or up
                let going = if v > row { "│ " } else { "│ " };
                let t = row as f64 / max_s as f64;
                let (r,g,b) = viridis(t);
                print!("{}", fg(r,g,b, going));
            } else if i > 0 && step_vals[i] != row && step_vals[i-1] != row {
                // Check if vertical line passes through this row
                let prev = step_vals[i-1];
                let cur = step_vals[i];
                let (lo, hi) = if prev < cur { (prev, cur) } else { (cur, prev) };
                if row > lo && row < hi && i > 0 {
                    let t = row as f64 / max_s as f64;
                    let (r,g,b) = viridis(t);
                    print!("{}", fg(r,g,b, "│ "));
                } else {
                    print!("  ");
                }
            } else {
                print!("  ");
            }
        }
        println!();
    }
    println!("     \x1b[2m  └──────────────────────────────\x1b[0m");
    println!();

    // N5: Event timeline / marker chart
    sub("N5", "Event Timeline / Markers", "Discrete events on a continuous axis");
    print!("     \x1b[2m│\x1b[0m");
    let events: [(usize, &str, (u8,u8,u8)); 6] = [
        (3, "▼", (220,60,60)),
        (8, "▼", (240,200,60)),
        (12,"▼", (80,200,120)),
        (18,"▼", (100,180,255)),
        (25,"▼", (220,60,60)),
        (33,"▼", (200,100,200)),
    ];
    let tl_w = 40;
    let mut tl_line = vec![("─", (80,80,80)); tl_w];
    for (pos, marker, color) in &events {
        if *pos < tl_w { tl_line[*pos] = (*marker, *color); }
    }
    for (ch, (r,g,b)) in &tl_line {
        print!("{}", fg(*r,*g,*b, ch));
    }
    println!("\x1b[2m│\x1b[0m");
    print!("     ");
    for (pos, _, _) in &events {
        let label_pos = if *pos > 1 { *pos } else { 1 };
        print!("{:>w$}", "│", w=if label_pos > 0 {3} else {1});
    }
    println!();
    println!("     \x1b[2m release   bug    feature  refactor  bug    deploy\x1b[0m");
    println!();
}

// =====================================================================
// O. TEXT-INTEGRATED VISUALIZATIONS
// =====================================================================

fn section_text_integrated_viz() {
    heading("O", "TEXT-INTEGRATED VISUALIZATIONS");

    // O1: Data bars in table cells
    sub("O1", "In-Cell Data Bars", "Bars embedded within table cells — like Excel conditional formatting");
    println!("     ╭──────────────────┬────────┬──────────────────────╮");
    println!("     │ Function         │  Score │ Distribution         │");
    println!("     ├──────────────────┼────────┼──────────────────────┤");
    let table_data = [
        ("parse_expr",      42, 42),
        ("resolve_imports", 31, 42),
        ("build_graph",     24, 42),
        ("validate_ast",    18, 42),
    ];
    for (name, val, max_v) in &table_data {
        let t = *val as f64 / *max_v as f64;
        let (r,g,b) = heat(t);
        let bar_w = (t * 20.0) as usize;
        let bar = fg(r,g,b, &"█".repeat(bar_w));
        let pad = " ".repeat(20 - bar_w);
        println!("     │ {:<16} │ {:>6} │ {}{} │",
            name, fg(r,g,b,&val.to_string()), bar, pad);
    }
    println!("     ╰──────────────────┴────────┴──────────────────────╯");
    println!();

    // O2: Heatmap-colored text
    sub("O2", "Heatmap-Colored Text", "The text IS the visualization — value encoded in text color");
    let funcs = [
        ("parse_expr",     42), ("resolve_imports",31), ("build_graph",24),
        ("validate_ast",   18), ("emit_warning",    7), ("new_scope",  12),
        ("check_types",    28), ("fold_const",     15), ("inline_fn",   9),
    ];
    println!("     \x1b[2mFunction names colored by complexity score:\x1b[0m");
    print!("     ");
    for (i, (name, val)) in funcs.iter().enumerate() {
        let t = *val as f64 / 42.0;
        let (r,g,b) = heat(t);
        print!("{}", fg(r,g,b, name));
        if i < funcs.len() - 1 { print!("  "); }
        if (i + 1) % 3 == 0 { print!("\n     "); }
    }
    println!();
    println!();

    // O3: Inline sparklines in table
    sub("O3", "Inline Sparklines in Table Cells", "Trend mini-charts within each row");
    let blocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    println!("     ╭──────────────┬───────┬────────────────────┬───────╮");
    println!("     │ Module       │  CC   │ Trend (10 commits) │ Δ     │");
    println!("     ├──────────────┼───────┼────────────────────┼───────┤");
    let sparkdata: [(&str, f64, [u8;10], &str); 4] = [
        ("render",     8.3, [3,4,4,5,5,6,6,7,7,8], "+2.1↑"),
        ("deps",      12.1, [8,7,7,6,6,5,5,4,4,3], "-3.2↓"),
        ("complexity", 6.7, [5,5,5,6,5,5,6,5,5,5], " 0.0→"),
        ("loc",        4.2, [2,3,4,3,4,5,4,3,4,4], "+0.3↑"),
    ];
    for (name, cc, trend, delta) in &sparkdata {
        let cc_t = *cc / 15.0;
        let (cr,cg,cb) = heat(cc_t);
        print!("     │ {:<12} │ {} │ ", name, fg(cr,cg,cb, &format!("{:>5.1}", cc)));
        for &v in trend {
            let t = v as f64 / 8.0;
            let (r,g,b) = viridis(t);
            print!("{}", fg(r,g,b, &blocks[(v as usize - 1).min(7)].to_string()));
        }
        let delta_color = if delta.contains('↑') { fg(220,60,60,delta) }
            else if delta.contains('↓') { fg(80,200,120,delta) }
            else { fg(150,150,150,delta) };
        println!("          │ {} │", delta_color);
    }
    println!("     ╰──────────────┴───────┴────────────────────┴───────╯");
    println!();

    // O4: Conditional row highlighting
    sub("O4", "Conditional Row Highlighting", "Entire row background changes based on value/status");
    println!("     \x1b[2m  Function              CC   Status\x1b[0m");
    let rows = [
        ("parse_expr",     42, "critical"),
        ("resolve_imports",31, "warning"),
        ("build_graph",    24, "warning"),
        ("validate_ast",   18, "ok"),
        ("emit_warning",    7, "ok"),
    ];
    for (name, cc, status) in &rows {
        let (br,bgg,bb) = match *status {
            "critical" => (60,20,20),
            "warning"  => (50,40,15),
            _          => (20,20,20),
        };
        let (fr,fgg,fb) = match *status {
            "critical" => (255,100,100),
            "warning"  => (240,200,80),
            _          => (180,220,180),
        };
        let line = format!("  {:<22} {:>3}   {:<10}", name, cc, status);
        println!("     {}", fgbg(fr,fgg,fb, br,bgg,bb, &line));
    }
    println!();

    // O5: Annotation callouts
    sub("O5", "Annotation / Callout Lines", "Point at specific data with explanatory text");
    let bar_vals = [3,7,5,12,4,6,2,9];
    let max_b = 12;
    print!("     ");
    for &v in &bar_vals {
        let t = v as f64 / max_b as f64;
        let (r,g,b) = viridis(t);
        print!("{}", fg(r,g,b, &format!("{:>3} ", v)));
    }
    println!();
    print!("     ");
    for &v in &bar_vals {
        let t = v as f64 / max_b as f64;
        let h = (t * 6.0) as usize;
        let (r,g,b) = viridis(t);
        let bks = ['▁','▂','▃','▄','▅','▆','▇','█'];
        print!("{}", fg(r,g,b, &format!(" {} ", bks[h.min(7)])));
        print!(" ");
    }
    println!();
    // Callout arrow pointing to the outlier (12)
    println!("     \x1b[2m            ↑\x1b[0m");
    println!("     \x1b[2m            ╰── {} (z=2.4, outlier)\x1b[0m", fg(253,231,37, "12"));
    println!();

    // O6: Multi-metric compact row
    sub("O6", "Multi-Metric Compact Row", "All metrics for one item in a single dense line");
    println!("     \x1b[2mformat: name  CC▕gauge▏ nest▕gauge▏ deps▕gauge▏ trend  grade\x1b[0m");
    let compact = [
        ("parse_expr",     8.5, 4, 6, [3,4,5,6,7,8], "C"),
        ("resolve_imports",3.2, 2, 3, [3,3,3,3,3,3], "A"),
        ("build_graph",    5.1, 3, 8, [2,3,4,5,6,5], "B"),
    ];
    let blocks = ['▁','▂','▃','▄','▅','▆','▇','█'];
    for (name, cc, nest, deps, trend, grade) in &compact {
        let cc_t = *cc / 10.0;
        let nest_t = *nest as f64 / 6.0;
        let deps_t = *deps as f64 / 10.0;
        let (cr,cg,cb) = heat(cc_t);
        let (nr,ng,nb) = heat(nest_t);
        let (dr,dg,db) = heat(deps_t);
        print!("     {:<16}", name);
        // CC gauge
        let cc_bar = (cc_t * 5.0) as usize;
        print!(" {} {}{}\x1b[2m{}\x1b[0m",
            fg(cr,cg,cb, &format!("{:.1}", cc)),
            fg(cr,cg,cb, &"█".repeat(cc_bar)),
            "\x1b[2m░\x1b[0m".repeat(5-cc_bar),
            ""
        );
        // Nest gauge
        let n_bar = (nest_t * 4.0) as usize;
        print!(" {} {}{}",
            fg(nr,ng,nb, &format!("n{}", nest)),
            fg(nr,ng,nb, &"█".repeat(n_bar)),
            "\x1b[2m░\x1b[0m".repeat(4-n_bar),
        );
        // Deps
        print!(" {} ", fg(dr,dg,db, &format!("d{}", deps)));
        // Trend sparkline
        for &v in trend {
            let t = v as f64 / 8.0;
            let (r,g,b) = viridis(t);
            print!("{}", fg(r,g,b, &blocks[(v as usize -1).min(7)].to_string()));
        }
        // Grade badge
        let grade_color = match *grade {
            "A" => (80,200,120),
            "B" => (240,200,60),
            _ => (220,60,60),
        };
        print!(" {}", fgbg(0,0,0, grade_color.0,grade_color.1,grade_color.2, &format!(" {} ", grade)));
        println!();
    }
    println!();

    println!("\n{}", "═".repeat(70));
    println!("  END OF GLOSSARY");
    println!("  A: 8 pixel primitives    B: 8 line styles      C: 7 color modes");
    println!("  D: 9 palettes            E: 12 chart elements  F: 9 symbol sets");
    println!("  G: 7 layout patterns     H: 7 terminal features");
    println!("  I: 6 distribution viz    J: 7 comparison viz   K: 6 relational viz");
    println!("  L: 4 matrix/grid viz     M: 6 part-to-whole    N: 5 temporal viz");
    println!("  O: 6 text-integrated     = {} total entries", 8+8+7+9+12+9+7+7+6+7+6+4+6+5+6);
    println!("  Run: rustc demo_glossary.rs -o demo_glossary && ./demo_glossary");
    println!("{}\n", "═".repeat(70));
}

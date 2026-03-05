//! Generates the Ferrograph logo mark as a metaball-style graph SVG.
//! Uses bezier bridges between circle nodes (Varun Vachhar / Paper.js style).

use clap::Parser;

#[derive(Debug, Clone, Copy)]
struct Circle {
    x: f64,
    y: f64,
    r: f64,
}

#[derive(Debug, Clone, Copy)]
struct Edge(usize, usize);

#[derive(Parser)]
#[command(about = "Generate Ferrograph metaball logo SVG")]
struct Args {
    /// Spread factor for bridge curvature (0.0–1.0, default 0.5)
    #[arg(long, default_value = "0.5")]
    spread: f64,
    /// Bezier handle length (default 8.0)
    #[arg(long, default_value = "8.0")]
    handle: f64,
    /// Viewbox size (default 32)
    #[arg(long, default_value = "32")]
    size: u32,
}

fn main() {
    let args = Args::parse();
    let (nodes, edges) = default_layout();
    let svg = generate_svg(&nodes, &edges, args.spread, args.handle, args.size);
    println!("{svg}");
}

/// Default 3-cluster graph: 3 anchors + 6 satellites (9 nodes), 12 edges.
/// Organic, asymmetric layout so the mark feels hand-placed rather than geometric.
fn default_layout() -> (Vec<Circle>, Vec<Edge>) {
    let nodes = vec![
        // Cluster A (top-left): anchor 0, satellites 1, 2
        Circle {
            x: 7.8,
            y: 11.2,
            r: 2.9,
        }, // 0
        Circle {
            x: 4.8,
            y: 8.6,
            r: 2.0,
        }, // 1
        Circle {
            x: 10.3,
            y: 8.4,
            r: 2.25,
        }, // 2
        // Cluster B (bottom-center): anchor 3, satellites 4, 5
        Circle {
            x: 15.5,
            y: 19.8,
            r: 3.1,
        }, // 3
        Circle {
            x: 12.0,
            y: 23.2,
            r: 2.0,
        }, // 4
        Circle {
            x: 19.4,
            y: 22.0,
            r: 2.15,
        }, // 5
        // Cluster C (top-right): anchor 6, satellites 7, 8
        Circle {
            x: 24.2,
            y: 11.6,
            r: 2.85,
        }, // 6
        Circle {
            x: 27.4,
            y: 8.9,
            r: 2.1,
        }, // 7
        Circle {
            x: 21.6,
            y: 8.8,
            r: 2.2,
        }, // 8
    ];
    let edges = vec![
        Edge(0, 1),
        Edge(0, 2),
        Edge(1, 2),
        Edge(3, 4),
        Edge(3, 5),
        Edge(4, 5),
        Edge(6, 7),
        Edge(6, 8),
        Edge(7, 8),
        Edge(0, 3),
        Edge(0, 6),
        Edge(3, 6),
    ];
    (nodes, edges)
}

/// Generates the full SVG document.
fn generate_svg(
    nodes: &[Circle],
    edges: &[Edge],
    v: f64,
    handle_size: f64,
    view_size: u32,
) -> String {
    let mut paths = String::new();

    // Bridges first (so circles draw on top for cleaner overlap)
    for edge in edges {
        let (i, j) = (edge.0, edge.1);
        if i < nodes.len() && j < nodes.len() {
            if let Some(path) = metaball_bridge(&nodes[i], &nodes[j], v, handle_size) {
                paths.push_str(&path);
            }
        }
    }

    // Circles
    for c in nodes {
        paths.push_str(&circle_path(c));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_size} {view_size}" fill="none" aria-hidden="true">
  <title>Ferrograph logo</title>
  <!-- Metaball graph mark: programmatically generated. Regenerate with: cd assets/gen-logo && cargo run > ../logo.svg -->
  <defs>
    <filter id="goo" x="-10%" y="-10%" width="120%" height="120%">
      <feGaussianBlur in="SourceGraphic" stdDeviation="1.0" result="blur"/>
      <feColorMatrix in="blur" type="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 30 -12" result="goo"/>
    </filter>
  </defs>
  <g id="mark" class="mark" filter="url(#goo)">
    {paths}
  </g>
  <style>
    .mark path {{ fill: var(--fg-body, currentColor); }}
  </style>
</svg>
"#,
        view_size = view_size,
        paths = paths
    )
}

/// SVG path for a single circle.
fn circle_path(c: &Circle) -> String {
    // Full circle as path: M cx+r,cy A r,r 0 1,1 cx-r,cy A r,r 0 1,1 cx+r,cy Z
    format!(
        r#"    <path d="M {} {} A {} {} 0 1 1 {} {} A {} {} 0 1 1 {} {} Z" />"#,
        c.x + c.r,
        c.y,
        c.r,
        c.r,
        c.x - c.r,
        c.y,
        c.r,
        c.r,
        c.x + c.r,
        c.y
    )
}

/// Bezier bridge between two circles (metaball membrane). Returns SVG path fragment or None if circles too far.
fn metaball_bridge(c1: &Circle, c2: &Circle, v: f64, handle_size: f64) -> Option<String> {
    let dx = c2.x - c1.x;
    let dy = c2.y - c1.y;
    let d = (dx * dx + dy * dy).sqrt();
    if d < 1e-6 {
        return None;
    }
    let (r1, r2) = (c1.r, c2.r);
    // No bridge if circles are too far apart (threshold ~2.5 * sum of radii)
    if d > (r1 + r2) * 2.5 {
        return None;
    }

    let angle = dy.atan2(dx);
    let ratio = (r1 - r2) / d;
    let max_spread = ratio.clamp(-1.0, 1.0).acos();
    let spread = max_spread * v.clamp(0.0, 1.0);

    let cos_a_plus = (angle + spread).cos();
    let sin_a_plus = (angle + spread).sin();
    let cos_a_minus = (angle - spread).cos();
    let sin_a_minus = (angle - spread).sin();

    let p1_a_x = c1.x + r1 * cos_a_plus;
    let p1_a_y = c1.y + r1 * sin_a_plus;
    let p1_b_x = c1.x + r1 * cos_a_minus;
    let p1_b_y = c1.y + r1 * sin_a_minus;
    let p2_a_x = c2.x - r2 * cos_a_plus;
    let p2_a_y = c2.y - r2 * sin_a_plus;
    let p2_b_x = c2.x - r2 * cos_a_minus;
    let p2_b_y = c2.y - r2 * sin_a_minus;

    let (v1_x, v1_y) = (p2_a_x - p1_a_x, p2_a_y - p1_a_y);
    let len1 = (v1_x * v1_x + v1_y * v1_y).sqrt();
    let (u1_x, u1_y) = if len1 > 1e-10 {
        (v1_x / len1, v1_y / len1)
    } else {
        (1.0, 0.0)
    };
    let cp1_a_x = p1_a_x + u1_x * handle_size;
    let cp1_a_y = p1_a_y + u1_y * handle_size;
    let cp2_a_x = p2_a_x - u1_x * handle_size;
    let cp2_a_y = p2_a_y - u1_y * handle_size;

    let (v2_x, v2_y) = (p1_b_x - p2_b_x, p1_b_y - p2_b_y);
    let len2 = (v2_x * v2_x + v2_y * v2_y).sqrt();
    let (u2_x, u2_y) = if len2 > 1e-10 {
        (v2_x / len2, v2_y / len2)
    } else {
        (1.0, 0.0)
    };
    let cp1_b_x = p2_b_x + u2_x * handle_size;
    let cp1_b_y = p2_b_y + u2_y * handle_size;
    let cp2_b_x = p1_b_x - u2_x * handle_size;
    let cp2_b_y = p1_b_y - u2_y * handle_size;

    let path = format!(
        r#"    <path d="M {} {} C {} {} {} {} {} {} L {} {} C {} {} {} {} {} {} Z" />"#,
        p1_a_x,
        p1_a_y,
        cp1_a_x,
        cp1_a_y,
        cp2_a_x,
        cp2_a_y,
        p2_a_x,
        p2_a_y,
        p2_b_x,
        p2_b_y,
        cp1_b_x,
        cp1_b_y,
        cp2_b_x,
        cp2_b_y,
        p1_b_x,
        p1_b_y
    );
    Some(path)
}

//! Cut a play area out of an Overture segment file and write it as GeoJSON for the browser.
//!
//! ```sh
//! cargo run --release -p overrules-city-prep -- <segment.parquet> <out.geojson> <west> <south> <east> <north>
//! ```

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

use geo::BoundingRect;
use overrules::io::geojson;
use serde_json::Value;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, w, s, e, n] = args.as_slice() else {
        return Err(
            "usage: overrules-city-prep <segments> <out.geojson> <west> <south> <east> <north>"
                .into(),
        );
    };
    let (w, s, e, n): (f64, f64, f64, f64) = (w.parse()?, s.parse()?, e.parse()?, n.parse()?);

    let segments = overrules::io::read_path(input)?;
    let total = segments.len();
    let kept: Vec<_> = segments
        .iter()
        .filter(|seg| seg.subtype.as_deref() == Some("road"))
        .filter(|seg| {
            seg.geometry().bounding_rect().is_some_and(|r| {
                r.min().x >= w && r.max().x <= e && r.min().y >= s && r.max().y <= n
            })
        })
        .collect();

    let mut fc = geojson::to_feature_collection(kept.iter().copied());
    round_coordinates(&mut fc);

    let mut out = BufWriter::new(File::create(output)?);
    serde_json::to_writer(&mut out, &fc)?;
    out.flush()?;
    eprintln!("kept {} of {total} segments → {output}", kept.len());
    Ok(())
}

/// Six decimal places (~10 cm) is plenty for the map and halves the file.
fn round_coordinates(fc: &mut Value) {
    let Some(features) = fc.get_mut("features").and_then(Value::as_array_mut) else {
        return;
    };
    for f in features {
        let Some(coords) = f
            .pointer_mut("/geometry/coordinates")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for c in coords.iter_mut().filter_map(Value::as_array_mut) {
            for v in c.iter_mut() {
                if let Some(x) = v.as_f64() {
                    *v = Value::from((x * 1e6).round() / 1e6);
                }
            }
        }
    }
}

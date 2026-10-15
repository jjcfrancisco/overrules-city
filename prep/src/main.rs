//! Cut a play area out of an Overture segment file and write it, slimmed and gzipped, as the
//! GeoJSON the page loads.
//!
//! ```sh
//! cargo run --release -p overrules-city-prep -- <segment.parquet> <out.json.gz> <west> <south> <east> <north>
//! ```

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

use flate2::Compression;
use flate2::write::GzEncoder;
use geo::BoundingRect;
use overrules::io::geojson;
use serde_json::Value;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, w, s, e, n] = args.as_slice() else {
        return Err(
            "usage: overrules-city-prep <segments> <out.json.gz> <west> <south> <east> <north>"
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
    slim(&mut fc);

    let json = serde_json::to_vec(&fc)?;
    let mut out = GzEncoder::new(BufWriter::new(File::create(output)?), Compression::best());
    out.write_all(&json)?;
    out.finish()?.flush()?;
    let gz = std::fs::metadata(output)?.len();
    eprintln!(
        "kept {} of {total} segments → {output} ({:.1} MB of JSON, {:.2} MB gzipped)",
        kept.len(),
        json.len() as f64 / 1e6,
        gz as f64 / 1e6
    );
    Ok(())
}

/// Drops what the page never reads and rounds coordinates to six decimal places (~10 cm).
fn slim(fc: &mut Value) {
    let Some(features) = fc.get_mut("features").and_then(Value::as_array_mut) else {
        return;
    };
    for f in features {
        let Some(obj) = f.as_object_mut() else {
            continue;
        };
        // The id is kept in the properties, where the reader looks first.
        obj.remove("id");
        if let Some(props) = obj.get_mut("properties").and_then(Value::as_object_mut) {
            props.remove("routes");
            props.retain(|_, v| {
                !matches!(v, Value::Null) && v.as_array().is_none_or(|a| !a.is_empty())
            });
        }
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

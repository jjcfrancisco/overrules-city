# overrules · live city

A live Lower Manhattan where about 600 cars, vans, lorries, bikes and pedestrians follow
Overture Maps' real transport rules, evaluated in the browser by
[`overrules`](https://github.com/jjcfrancisco/overrules) (Rust, compiled to WebAssembly).
It runs on its own: a tour of numbered scenes flies to real streets and shows the library
answering live, rule by rule.

## What you see

- **The city.** At every junction each traveller asks overrules about every way out: may I go
  there, how fast, is this turn banned? It picks a legal way at random. Yellow rings are
  refusals; the live feed says who refused: `OVERRULES` (the data) or a `GAME RULE` (where
  the data is silent).
- **The tour.** It runs by itself; ← → step through it, space pauses, and `#03` in the address
  opens a scene directly.

  | # | Street | What it shows |
  |---|---|---|
  | 01 | the whole area | every junction, every way out |
  | 02 | City Hall Park Greenway | the last matching rule decides |
  | 03 | East 14th Street | a time window (the busway) opening at 22:00 |
  | 04 | FDR Drive | the travel-mode tree: car, truck and hgv under motor vehicle |
  | 05 | City Hall Park Greenway | "can't be sure" when the time isn't given |
  | 06 | Brooklyn Bridge | vehicle size: height and weight must both hold |
  | 07 | Canal Street | a one-step and a two-step turn ban |
  | 08 | Rivington Street | malformed opening hours, repaired |
  | 09 | Cherry Street | flat-map maths would move a rule boundary by 11 m |
  | 10 | the whole area | rules evaluated per second, in the browser |
  | 11 | wherever it happens | the latest refusals, live |

- **Explore.** Click any road for every rule in plain English, which one decides, and a what-if
  grid: car, van, lorry, "some motor vehicle", bike and on foot, now, Friday 17:30, Sunday
  03:00 or with the time unknown, claiming a purpose or permit. Click any dot to see its last
  decision.

## Run it

Needs Rust, [`just`](https://github.com/casey/just), [DuckDB](https://duckdb.org), Node (for
`npm pack`) and Python 3. WebAssembly builds on macOS also need LLVM (`brew install llvm`).

```sh
just play    # download what's missing, build, and serve at http://localhost:8790
```

That runs, as needed:

```sh
just data    # Lower Manhattan's segments from Overture's S3 bucket → data/segment.parquet
just prep    # → web/data/area.geojson, the file the page loads
just vendor  # MapLibre GL JS and the fonts → web/vendor
just wasm    # the city, compiled to WebAssembly → web/pkg
```

`just test` runs the native tests, including one that checks every claim the tour makes on
screen against the data. `just check` adds formatting and lints.

## How it works

| piece | what |
|---|---|
| `city/src/engine.rs` | The road graph (one step per stretch between connectors), travellers, and the checks that turn overrules answers into moves. |
| `city/src/sim.rs` | The travellers and the feed of refusals. |
| `city/src/signs.rs` | Rules drawn on the map for one vehicle at one moment. |
| `city/src/inspect.rs` | The road panel, the what-if grid, and the tour's questions. |
| `city/src/lib.rs` | The browser API (`City`), returning JSON. |
| `prep/` | Cuts the play area out of an Overture segment file. |
| `web/` | The page: MapLibre with OpenFreeMap tiles, recoloured; styled after [xerj.org](https://xerj.org). |

Each move is checked like this:

- **Access:** `overrules::resolve` at three points along the stretch.
- **Turns:** the `prohibited_transitions` of the last few roads, matched against the steps
  taken, including multi-step sequences.
- **Speed:** the winning speed-limit rule.

Where the data is silent, overrules deliberately doesn't guess, so the city adds a few labelled
**game rules**: footways are for walkers; a one-way that names no travel mode doesn't stop
people on foot; "designated for lorries" doesn't let lorries go the wrong way down a one-way; a
road whose rules only let some people in is closed to the rest; 25 mph where no limit is given.

## Licence

Code: [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Overture
transportation data is © OpenStreetMap contributors and Overture Maps Foundation, under the
[ODbL](https://opendatacommons.org/licenses/odbl/1-0/). Map tiles: [OpenFreeMap](https://openfreemap.org),
© OpenMapTiles, data from OpenStreetMap. Fonts are bundled from Fontsource under their own
licences (SIL Open Font License).

//! Compare two music database snapshots table by table.
//! Usage: `cargo run --bin db-diff -- <A dir or .sqlitedb> <B dir or .sqlitedb>`

use std::path::PathBuf;

use miu_music_lib::sync::backup::DB_NAME;
use miu_music_lib::sync::dbdiff::diff;

fn main() {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let [a, b] = args.as_slice() else {
        eprintln!("usage: db-diff <A> <B>   (backup folders or .sqlitedb files)");
        std::process::exit(2);
    };
    let file = |p: &PathBuf| if p.is_dir() { p.join(DB_NAME) } else { p.clone() };
    match diff(&file(a), &file(b)) {
        Ok(d) if d.is_empty() => println!("no differences"),
        Ok(d) => d.tables.iter().for_each(|t| println!("{}", t.summary())),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

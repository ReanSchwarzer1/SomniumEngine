use mlua::Table;
use std::time::Instant;

fn main() {
    let lua = somnium_script_luau::new_sandboxed_state(64 * 1024 * 1024).unwrap();
    let n = 200_000u32;
    let ns = |d: std::time::Duration| d.as_nanos() as f64 / f64::from(n);

    let t: Table = lua.create_table().unwrap();
    t.set("translation", mlua::Vector::new(1.0, 2.0, 3.0))
        .unwrap();
    let key = lua.create_string("translation").unwrap();
    let v = mlua::Vector::new(1.0, 2.0, 3.0);

    let s = Instant::now();
    for _ in 0..n {
        t.set("translation", v).unwrap();
    }
    println!("Table::set(&str)      {:>7.1} ns", ns(s.elapsed()));

    let s = Instant::now();
    for _ in 0..n {
        t.raw_set("translation", v).unwrap();
    }
    println!("Table::raw_set(&str)  {:>7.1} ns", ns(s.elapsed()));

    let s = Instant::now();
    for _ in 0..n {
        t.raw_set(&key, v).unwrap();
    }
    println!("Table::raw_set(String){:>7.1} ns", ns(s.elapsed()));

    let s = Instant::now();
    for _ in 0..n {
        let _: mlua::Vector = t.get("translation").unwrap();
    }
    println!("Table::get(&str)      {:>7.1} ns", ns(s.elapsed()));

    let s = Instant::now();
    for _ in 0..n {
        let _: mlua::Vector = t.raw_get("translation").unwrap();
    }
    println!("Table::raw_get(&str)  {:>7.1} ns", ns(s.elapsed()));

    let s = Instant::now();
    for _ in 0..n {
        let _: mlua::Vector = t.raw_get(&key).unwrap();
    }
    println!("Table::raw_get(String){:>7.1} ns", ns(s.elapsed()));
}

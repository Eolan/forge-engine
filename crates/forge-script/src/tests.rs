use std::time::{Duration, Instant};

use super::*;

fn script() -> Script {
    Script::new(Limits::default()).unwrap()
}

fn map(entries: &[(&str, Data)]) -> Data {
    Data::Map(
        entries
            .iter()
            .map(|(k, v)| (Key::String((*k).to_owned()), v.clone()))
            .collect(),
    )
}

#[test]
fn a_returned_table_comes_back_as_data() {
    let out = script()
        .run(
            "data.lua",
            r#"return { name = "town", cells = { 1, 2, 3 }, scale = 0.5, nested = { a = { b = true } } }"#,
        )
        .unwrap();
    assert_eq!(
        out,
        map(&[
            (
                "cells",
                Data::List(vec![Data::Integer(1), Data::Integer(2), Data::Integer(3)])
            ),
            ("name", Data::String("town".into())),
            ("nested", map(&[("a", map(&[("b", Data::Bool(true))]))])),
            ("scale", Data::Number(0.5)),
        ])
    );
}

#[test]
fn tables_built_by_code_come_back_as_globals() {
    // Lua 5.1 as data files write it: a global table filled in a loop, a local helper.
    let mut s = script();
    s.run(
        "rules.lua",
        r#"
        local function weight(i) return i * 0.5 end
        Items = {}
        for i = 1, 10 do
            Items[i] = { id = "item" .. i, weight = weight(i) }
        end
        Count = #Items
        "#,
    )
    .unwrap();
    let globals = s.globals().unwrap();
    assert_eq!(globals["Count"], Data::Integer(10));
    let Data::List(items) = &globals["Items"] else {
        panic!("{:?}", globals["Items"]);
    };
    assert_eq!(items.len(), 10);
    assert_eq!(
        items[2],
        map(&[
            ("id", Data::String("item3".into())),
            ("weight", Data::Number(1.5))
        ])
    );
}

#[test]
fn a_megabyte_of_data_table_reads_in_well_under_a_second() {
    // A data file's shape: a big table of records, each a few fields.
    let mut text = String::from("Records = {\n");
    let mut n = 0;
    while text.len() < 1 << 20 {
        text.push_str(&format!(
            "    {{ name = \"thing_{n}\", category = \"cat{}\", weight = {}.25, tags = {{ \"a\", \"b{}\" }} }},\n",
            n % 17,
            n % 9,
            n % 5
        ));
        n += 1;
    }
    text.push_str("}\n");
    let mut s = script();
    let start = Instant::now();
    s.run("records.lua", &text).unwrap();
    let globals = s.globals().unwrap();
    let took = start.elapsed();
    let Data::List(records) = &globals["Records"] else {
        panic!()
    };
    assert_eq!(records.len(), n);
    println!(
        "{} bytes, {n} records: {:.1} ms, {} interrupts, {} bytes of memory",
        text.len(),
        took.as_secs_f64() * 1e3,
        s.spent(),
        s.memory()
    );
    assert!(took < Duration::from_secs(1), "{took:?}");
}

#[test]
fn the_sandbox_has_no_io_os_debug_or_loaders() {
    let out = script()
        .run(
            "probe.lua",
            r#"
            local names = { "io", "os", "debug", "package" }
            local out = {}
            for _, n in ipairs(names) do out[n] = type(_G[n]) end
            return out
            "#,
        )
        .unwrap();
    let Data::Map(found) = out else { panic!() };
    for (k, v) in &found {
        assert_eq!(*v, Data::String("nil".into()), "{k:?}");
    }
    assert_eq!(found.len(), 4);
    // The loaders and 5.1's environment functions say what they are.
    for name in [
        "load",
        "loadstring",
        "dofile",
        "loadfile",
        "require",
        "getfenv",
        "setfenv",
        "module",
    ] {
        let e = script()
            .run("probe.lua", &format!("return {name}(1)"))
            .unwrap_err();
        assert!(
            e.to_string()
                .contains(&format!("{name} is not available in this sandbox")),
            "{name}: {e}"
        );
    }
    // The libraries are read-only.
    let e = script().run("poke.lua", "string.upper = nil").unwrap_err();
    assert!(matches!(e, Error::Runtime(_)), "{e}");
    assert!(e.to_string().contains("readonly"), "{e}");
}

#[test]
fn a_runaway_chunk_ends_on_its_budget_and_the_host_goes_on() {
    let mut s = Script::new(Limits {
        interrupts: 100_000,
        ..Limits::default()
    })
    .unwrap();
    let e = s.run("loop.lua", "while true do end").unwrap_err();
    assert!(
        matches!(
            e,
            Error::Budget {
                what: "interrupts",
                ..
            }
        ),
        "{e}"
    );
    // The deadline, with interrupts to spare.
    let mut s = Script::new(Limits {
        interrupts: u64::MAX,
        deadline: Duration::from_millis(50),
        ..Limits::default()
    })
    .unwrap();
    let start = Instant::now();
    let e = s.run("loop.lua", "while true do end").unwrap_err();
    assert!(
        matches!(
            e,
            Error::Budget {
                what: "deadline",
                ..
            }
        ),
        "{e}"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    // The same state runs the next chunk.
    assert_eq!(s.run("next.lua", "return 1 + 1").unwrap(), Data::Integer(2));
}

#[test]
fn a_memory_bomb_ends_on_its_cap() {
    let mut s = Script::new(Limits {
        memory: 16 << 20,
        ..Limits::default()
    })
    .unwrap();
    let e = s
        .run(
            "bomb.lua",
            "local t = {} for i = 1, 100000000 do t[i] = string.rep('x', 64) .. i end",
        )
        .unwrap_err();
    assert!(matches!(e, Error::Memory { .. }), "{e}");
}

#[test]
fn endless_recursion_is_an_error_not_a_crash() {
    let e = script()
        .run(
            "deep.lua",
            "local function f(n) return f(n + 1) + 1 end return f(1)",
        )
        .unwrap_err();
    assert!(matches!(e, Error::Runtime(_) | Error::Budget { .. }), "{e}");
}

#[test]
fn errors_name_their_chunk_and_line() {
    let e = script().run("rules.lua", "x = = 1").unwrap_err();
    assert!(matches!(e, Error::Syntax(_)), "{e}");
    assert!(e.to_string().contains("rules.lua:1"), "{e}");
    let e = script()
        .run("rules.lua", "local a = nil\nreturn a.b")
        .unwrap_err();
    assert!(matches!(e, Error::Runtime(_)), "{e}");
    assert!(e.to_string().contains("rules.lua:2"), "{e}");
}

#[test]
fn stub_globals_let_code_written_for_another_engine_run() {
    let mut s = script();
    s.set("Game", &map(&[("version", Data::Integer(41))]))
        .unwrap();
    s.function("getText", |args| match args.first() {
        Some(Data::String(key)) => Ok(Data::String(format!("<{key}>"))),
        _ => Err("getText takes a key".to_owned()),
    })
    .unwrap();
    s.run(
        "stubs.lua",
        r#"
        Label = getText("IGUI_Name") .. " v" .. Game.version
        Ok, Message = pcall(getText, 3)
        "#,
    )
    .unwrap();
    let globals = s.globals().unwrap();
    assert_eq!(globals["Label"], Data::String("<IGUI_Name> v41".into()));
    assert_eq!(globals["Ok"], Data::Bool(false));
    assert!(!globals.contains_key("Game") && !globals.contains_key("getText"));
}

#[test]
fn require_reads_one_folder_only() {
    let root = std::env::temp_dir().join(format!("forge-script-{}", std::process::id()));
    std::fs::create_dir_all(root.join("mods")).unwrap();
    std::fs::write(
        root.join("mods/util.lua"),
        "local M = {} function M.double(x) return x * 2 end return M",
    )
    .unwrap();
    let mut s = Script::new(Limits {
        require_root: Some(root.clone()),
        ..Limits::default()
    })
    .unwrap();
    let out = s
        .run(
            "main.lua",
            r#"local u = require("mods.util") return u.double(21) + require("mods/util").double(0)"#,
        )
        .unwrap();
    assert_eq!(out, Data::Integer(42));
    for bad in ["../secret", "/etc/passwd", "C:/x", "mods/../../x", ""] {
        let e = s
            .run("main.lua", &format!("return require({bad:?})"))
            .unwrap_err();
        assert!(
            e.to_string().contains("not a module of the folder"),
            "{bad}: {e}"
        );
    }
    std::fs::remove_dir_all(&root).ok();
    // Without a folder, no require.
    assert!(script().run("main.lua", "return require('x')").is_err());
}

#[test]
fn load_compiles_only_when_allowed_and_stays_in_the_sandbox() {
    assert!(script().run("l.lua", "return load('return 1')()").is_err());
    let mut s = Script::new(Limits {
        allow_load: true,
        ..Limits::default()
    })
    .unwrap();
    assert_eq!(
        s.run("l.lua", "return loadstring('return 2 + 3')()")
            .unwrap(),
        Data::Integer(5)
    );
    assert_eq!(
        s.run("l.lua", "return load('return type(io)')()").unwrap(),
        Data::String("nil".into())
    );
}

#[test]
fn lua_5_1_idioms_run_as_luau() {
    // Closures, varargs, metatables, string patterns, pcall and error, ipairs and pairs,
    // `unpack`: the 5.1 a data file's code uses.
    let out = script()
        .run(
            "idioms.lua",
            r##"
            local function count(...) return select("#", ...) end
            local Vec = {}
            Vec.__index = Vec
            Vec.__add = function(a, b) return setmetatable({ x = a.x + b.x }, Vec) end
            local v = setmetatable({ x = 1 }, Vec) + setmetatable({ x = 2 }, Vec)
            local words = {}
            for w in string.gmatch("north east  south", "%a+") do words[#words + 1] = w end
            local ok, err = pcall(function() error("bad tile", 0) end)
            local a, b = unpack({ 7, 8 })
            return { count(1, nil, 3), v.x, table.concat(words, ","), ok, err, a + b,
                     string.format("%03d|%s", 7, "x"), math.floor(7.9) }
            "##,
        )
        .unwrap();
    assert_eq!(
        out,
        Data::List(vec![
            Data::Integer(3),
            Data::Integer(3),
            Data::String("north,east,south".into()),
            Data::Bool(false),
            Data::String("bad tile".into()),
            Data::Integer(15),
            Data::String("007|x".into()),
            Data::Integer(7),
        ])
    );
}

#[test]
fn a_table_that_holds_itself_is_refused_not_looped() {
    let e = script()
        .run("cycle.lua", "local t = {} t.me = t return t")
        .unwrap_err();
    assert!(matches!(e, Error::Data(_)), "{e}");
}

#[test]
fn require_cycles_return_and_extensions_are_dropped() {
    let root = std::env::temp_dir().join(format!("forge-script-cycle-{}", std::process::id()));
    std::fs::create_dir_all(root.join("lib")).unwrap();
    // A module that requires its parent, which requires it back.
    std::fs::write(
        root.join("lib/parent.lua"),
        "local child = require('lib.child') return { name = 'parent', child = child.name }",
    )
    .unwrap();
    std::fs::write(
        root.join("lib/child.lua"),
        "local p = require('lib.parent') return { name = 'child', saw = type(p) }",
    )
    .unwrap();
    let mut s = Script::new(Limits {
        require_root: Some(root.clone()),
        ..Limits::default()
    })
    .unwrap();
    let out = s
        .run(
            "main.lua",
            "local p = require('lib/parent.lua') return p.name .. '/' .. p.child .. '/' .. require('lib.child').saw",
        )
        .unwrap();
    assert_eq!(out, Data::String("parent/child/boolean".into()));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn lua_5_1_names_and_globals_through_g() {
    let out = script()
        .run(
            "compat.lua",
            r#"
            local words = {}
            for w in string.gfind("a bb", "%a+") do words[#words + 1] = w end
            x = 6
            _G.y = 7
            local seen = 0
            for k in pairs(_G) do if k == "x" or k == "y" then seen = seen + 1 end end
            return { math.mod(7, 3), #words, _G.x, y, seen, type(_G.print), collectgarbage("count") > 0 }
            "#,
        )
        .unwrap();
    assert_eq!(
        out,
        Data::List(vec![
            Data::Integer(1),
            Data::Integer(2),
            Data::Integer(6),
            Data::Integer(7),
            Data::Integer(2),
            Data::String("function".into()),
            Data::Bool(true),
        ])
    );
}

#[test]
fn the_lua_5_1_mode_has_writable_libraries() {
    let mut s = Script::new(Limits {
        lua51: true,
        ..Limits::default()
    })
    .unwrap();
    s.run(
        "util.lua",
        r#"
        function string.trim(s) return (string.gsub(s, "^%s*(.-)%s*$", "%1")) end
        Trimmed = ("  hello  "):trim()
        _G.Count = 3
        "#,
    )
    .unwrap();
    let globals = s.globals().unwrap();
    assert_eq!(globals["Trimmed"], Data::String("hello".into()));
    assert_eq!(globals["Count"], Data::Integer(3));
    // The base libraries are not the chunk's globals.
    assert!(!globals.contains_key("string") && !globals.contains_key("_G"));
    // Still no io or os, and the caps hold.
    assert_eq!(
        s.run("p.lua", "return type(io) .. type(os)").unwrap(),
        Data::String("nilnil".into())
    );
}

#[test]
fn a_root_its_modules_require_back_runs_once() {
    let root = std::env::temp_dir().join(format!("forge-script-root-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // The root counts its runs in a global; its module requires it back.
    std::fs::write(
        root.join("main.lua"),
        "Runs = (Runs or 0) + 1 local part = require('part') return { part = part, runs = Runs }",
    )
    .unwrap();
    std::fs::write(
        root.join("part.lua"),
        "local m = require('main') return type(m)",
    )
    .unwrap();
    let mut s = Script::new(Limits {
        require_root: Some(root.clone()),
        ..Limits::default()
    })
    .unwrap();
    let out = s.run_module("main").unwrap();
    assert_eq!(
        out,
        map(&[
            ("part", Data::String("boolean".into())),
            ("runs", Data::Integer(1))
        ])
    );
    std::fs::remove_dir_all(&root).ok();
}

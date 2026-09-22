use std::collections::BTreeMap;

#[test]
fn counts() {
    let mut m: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for s in mcp_server_lib::spec::catalog() {
        m.entry(s.namespace).or_default().push(s.name);
    }
    let mut total = 0;
    for (ns, names) in &m {
        total += names.len();
        println!("{ns} ({}) : {}", names.len(), names.join(", "));
    }
    println!("TOTAL {total}");
}

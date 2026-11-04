//! End to end tests that exercise the embedded JVM bridge.

use bsh_runtime::Interpreter;
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

#[derive(Clone)]
struct Buf(Rc<RefCell<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn jvm_available() -> bool {
    bsh_jvm::start().is_ok()
}

/// Returns None when no JVM is available so the suite degrades gracefully.
fn run(src: &str) -> Option<(String, Result<(), String>)> {
    if !jvm_available() {
        return None;
    }
    let buf = Buf(Rc::new(RefCell::new(Vec::new())));
    let mut interp = Interpreter::new(Box::new(buf.clone()));
    let result = interp
        .eval_source(src)
        .map(|_| ())
        .map_err(|e| e.to_string());
    let out = String::from_utf8(buf.0.borrow().clone()).unwrap();
    Some((out, result))
}

fn check(src: &str, expected: &str) {
    match run(src) {
        Some((out, result)) => {
            if let Err(e) = result {
                panic!("script failed: {}\n{}", src, e);
            }
            assert_eq!(out, expected, "script: {}", src);
        }
        None => eprintln!("JVM unavailable, skipping JVM interop test"),
    }
}

#[test]
fn big_integer_calls() {
    check(
        r#"
        print(new java.math.BigInteger("12").add(new java.math.BigInteger("30")));
        print(new java.math.BigInteger("2").pow(10));
        print(java.math.BigInteger.valueOf(7).multiply(java.math.BigInteger.valueOf(6)));
    "#,
        "42\n1024\n42\n",
    );
}

#[test]
fn java_objects_and_fields() {
    check(
        r#"
        java.awt.Point p = new java.awt.Point(3, 4);
        print(p.x);
        print(p.y);
        p.x = 9;
        print(p.x);
        print(new java.util.Date().getTime() > 0);
        print(new java.io.File("/tmp").getName());
    "#,
        "3\n4\n9\ntrue\ntmp\n",
    );
}

#[test]
fn java_statics_and_properties() {
    check(
        r#"
        print(java.lang.System.getProperty("java.version") != null);
        print(java.lang.Integer.parseInt("123") + 1);
        print(java.lang.Math.max(2, 7));
        print(java.lang.String.valueOf(42));
    "#,
        "true\n124\n7\n42\n",
    );
}

#[test]
fn java_exceptions_are_catchable() {
    check(
        r#"
        try { new java.math.BigInteger("nope"); }
        catch (java.lang.NumberFormatException e) { print("caught number format"); }
        try { throw new java.lang.IllegalStateException("boom"); }
        catch (java.lang.Exception e) { print(e.getMessage()); }
    "#,
        "caught number format\nboom\n",
    );
}

#[test]
fn java_instances_and_strings() {
    check(
        r#"
        Object d = new java.util.Date();
        print(d instanceof java.util.Date);
        print(d instanceof java.lang.String);
        print(new java.lang.StringBuilder().append("a").append(1).toString());
        print("x".concat(String.valueOf(2)));
    "#,
        "true\nfalse\na1\nx2\n",
    );
}

#[test]
fn lambdas_as_java_functional_interfaces() {
    check(
        r#"
        java.util.function.Supplier s = () -> 42;
        print(s.get());
        java.util.function.Function f = x -> x * 2;
        print(f.apply(3));
        java.util.function.Predicate pr = x -> x > 2;
        print(pr.test(3));
        print(pr.test(1));
        print(java.util.Optional.of("y").map(v -> v + "!").get());
    "#,
        "42\n6\ntrue\nfalse\ny!\n",
    );
}

#[test]
fn scripted_objects_as_java_interfaces() {
    check(
        r#"
        java.lang.Runnable r = new java.lang.Runnable() {
            public void run() { print("ran"); }
        };
        r.run();
        new java.lang.Thread(r).run();
        java.util.Comparator c = new java.util.Comparator() {
            public int compare(Object a, Object b) { return ((String) a).compareTo((String) b); }
        };
        print(c.compare("a", "b") < 0);
        print(c.compare("b", "a") > 0);
    "#,
        "ran\nran\ntrue\ntrue\n",
    );
}

#[test]
fn default_method_composition() {
    check(
        r#"
        java.util.function.Function inc = x -> (int) x + 1;
        java.util.function.Function twice = inc.andThen(y -> (int) y * 2);
        print(twice.apply(3));
        java.util.function.Function ten = x -> x * 10;
        Object chained = ten.andThen(g -> (int) g + 5);
        print(chained.apply(2));
    "#,
        "8\n25\n",
    );
}

#[test]
fn method_references_as_functional_interfaces() {
    check(
        r#"
        java.util.function.Supplier s = java.util.ArrayList::new;
        print(s.get() instanceof java.util.List);
        java.util.function.Function upper = String::toUpperCase;
        print(upper.apply("abc"));
        java.util.function.Consumer c = java.lang.System.out::println;
        java.util.Optional.of("printed").ifPresent(c);
        java.util.function.Function valueOf = String::valueOf;
        print(valueOf.apply(42));
    "#,
        "true\nABC\nprinted\n42\n",
    );
}

#[test]
fn scripted_class_extends_java_thread() {
    check(
        r#"
        class MyThread extends java.lang.Thread {
            public void run() { print("custom run"); }
        }
        MyThread t = new MyThread();
        t.start();
        t.join();
        print(t instanceof java.lang.Thread);
    "#,
        "custom run\ntrue\n",
    );
}

#[test]
fn scripted_class_extends_java_collection() {
    check(
        r#"
        class MyList extends java.util.ArrayList {
            public int size() { return super.size() + 100; }
        }
        MyList l = new MyList();
        l.add("a");
        print(l.size());
        print(l instanceof java.util.ArrayList);
        print(java.util.Collections.max(l));
    "#,
        "101\ntrue\na\n",
    );
}

#[test]
fn scripted_subclass_fields_and_to_string() {
    check(
        r#"
        class MyPoint extends java.awt.Point {
            int tag;
            MyPoint(int x, int y) { this.x = x; this.y = y; this.tag = 7; }
            public String toString() { return "P(" + x + "," + y + ")"; }
        }
        MyPoint pt = new MyPoint(3, 4);
        print(pt.toString());
        print(pt.x);
        pt.x = 10;
        print(pt.x);
        print(pt.tag);
    "#,
        "P(3,4)\n3\n10\n7\n",
    );
}

#[test]
fn cross_thread_callbacks() {
    check(
        r#"
        java.lang.Runnable r = new java.lang.Runnable() {
            public void run() { print("thread ran"); }
        };
        java.lang.Thread t = new java.lang.Thread(r);
        t.start();
        print("main");
        t.join();
        print("done");
    "#,
        "main\nthread ran\ndone\n",
    );
}

#[test]
fn two_threads_share_the_interpreter() {
    check(
        r#"
        int total = 0;
        java.lang.Runnable r = new java.lang.Runnable() {
            public void run() { total = total + 1; }
        };
        java.lang.Thread a = new java.lang.Thread(r);
        java.lang.Thread b = new java.lang.Thread(r);
        a.start();
        b.start();
        a.join();
        b.join();
        print(total);
    "#,
        "2\n",
    );
}

#[test]
fn host_collections_cross_into_java() {
    check(
        r#"
        List l = new ArrayList();
        l.add("b");
        l.add("c");
        l.add("a");
        print(java.util.Collections.max(l));
        print(java.util.Collections.min(l));
        print(java.lang.String.join("-", l));
    "#,
        "c\na\nb-c-a\n",
    );
}

#[test]
fn java_collections_from_the_jvm() {
    check(
        r#"
        java.util.List l = new java.util.LinkedList();
        l.add("a");
        l.add("b");
        print(l.size());
        print(l.get(1));
        java.util.Map m = new java.util.TreeMap();
        m.put("k", 1);
        print(m.get("k"));
    "#,
        "2\nb\n1\n",
    );
}

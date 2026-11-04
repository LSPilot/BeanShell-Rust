//! End to end language tests: run BeanShell source and check the output.

use bsh_runtime::{Interpreter, Value};
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

fn run(src: &str) -> (String, Result<Value, String>) {
    let buf = Buf(Rc::new(RefCell::new(Vec::new())));
    let mut interp = Interpreter::new(Box::new(buf.clone()));
    let result = interp.eval_source(src).map_err(|e| e.to_string());
    let out = String::from_utf8(buf.0.borrow().clone()).unwrap();
    (out, result)
}

/// Run a script that must succeed and print exactly the expected text.
fn check(src: &str, expected: &str) {
    let (out, result) = run(src);
    match result {
        Ok(_) => assert_eq!(out, expected, "script: {}", src),
        Err(e) => panic!("script failed: {}\n{}", src, e),
    }
}

#[test]
fn arithmetic_and_promotion() {
    check(
        r#"
        print(1 + 2);
        print(7 / 2);
        print(7 % 3);
        print(7.0 / 2);
        print(1 << 4);
        print(1L << 40);
        print(5 & 3);
        print(true && false);
        print("a" + 1 + 2);
        print(1 + 2 + "a");
        print((int) 2.9);
        print((char) 65);
    "#,
        "3\n3\n1\n3.5\n16\n1099511627776\n1\nfalse\na12\n3a\n2\nA\n",
    );
}

#[test]
fn control_flow() {
    check(
        r#"
        int sum = 0;
        for (int i = 1; i <= 4; i++) sum += i;
        print(sum);
        int j = 0;
        while (j < 3) j++;
        print(j);
        do { j--; } while (j > 0);
        print(j);
        for (int x : new int[] {4, 5}) print(x);
        int k = 2;
        switch (k) { case 1: print("one"); break; case 2: print("two"); break; default: print("other"); }
        outer: for (int a = 0; a < 3; a++) { for (int b = 0; b < 3; b++) { if (b == 1) break outer; print(a + ":" + b); } }
    "#,
        "10\n3\n0\n4\n5\ntwo\n0:0\n",
    );
}

#[test]
fn methods_closures_and_recursion() {
    check(
        r#"
        int fact(int n) { if (n <= 1) return 1; return n * fact(n - 1); }
        print(fact(5));
        add(a, b) { return a + b; }
        print(add(2, 3));
        int base = 10;
        int addBase(int x) { return x + base; }
        print(addBase(5));
        sum(int... xs) { int t = 0; for (int x : xs) t += x; return t; }
        print(sum(1, 2, 3, 4));
    "#,
        "120\n5\n15\n10\n",
    );
}

#[test]
fn strings() {
    check(
        r#"
        String s = "Hello";
        print(s.length());
        print(s.substring(1, 3));
        print(s.toUpperCase());
        print(s.charAt(0));
        print(s.indexOf("l"));
        print(s.replace('l', 'L'));
        print(s.equals("Hello"));
        print("a,b,c".split(",").length);
        print(String.valueOf(42));
        print('x' + "");
        print("quote: \"q\"");
    "#,
        "5\nel\nHELLO\nH\n2\nHeLLo\ntrue\n3\n42\nx\nquote: \"q\"\n",
    );
}

#[test]
fn arrays() {
    check(
        r#"
        int[] a = new int[3];
        a[0] = 7;
        print(a[0]);
        print(a.length);
        int[] b = {1, 2, 3};
        print(b[2]);
        String[][] m = new String[2][2];
        m[0][0] = "x";
        print(m[0][0]);
        Object[] objs = new Object[] {1, "two"};
        print(objs[1]);
        print(int[].class != null);
    "#,
        "7\n3\n3\nx\ntwo\ntrue\n",
    );
}

#[test]
fn classes_and_inheritance() {
    check(
        r#"
        class Point {
            int x;
            int y;
            Point(int x, int y) { this.x = x; this.y = y; }
            int sum() { return x + y; }
            String toString() { return "Point(" + x + "," + y + ")"; }
        }
        Point p = new Point(2, 3);
        print(p.sum());
        print(p.toString());
        class Point3 extends Point {
            int z;
            Point3(int x, int y, int z) { super(x, y); this.z = z; }
            int sum() { return super.sum() + z; }
        }
        Point3 q = new Point3(1, 2, 3);
        print(q.sum());
        print(q instanceof Point);
        print(q instanceof Point3);
    "#,
        "5\nPoint(2,3)\n6\ntrue\ntrue\n",
    );
}

#[test]
fn exceptions() {
    check(
        r#"
        try { throw new RuntimeException("boom"); }
        catch (IllegalArgumentException e) { print("wrong"); }
        catch (RuntimeException e) { print(e.getMessage()); }
        finally { print("finally"); }
        try { int x = 1 / 0; } catch (e) { print("caught: " + e.getMessage()); }
        done() { try { return 1; } finally { print("cleanup"); } }
        print(done());
    "#,
        "boom\nfinally\ncaught: Division by zero\ncleanup\n1\n",
    );
}

#[test]
fn collections_and_builtins() {
    check(
        r#"
        List l = new ArrayList();
        l.add("a");
        l.add("b");
        print(l.size());
        print(l.get(1));
        print(l);
        Map m = new HashMap();
        m.put("k", 1);
        print(m.get("k"));
        print(m.containsKey("k"));
        StringBuilder sb = new StringBuilder();
        sb.append("x").append(1);
        print(sb.toString());
        print(Math.max(3, 9));
        print(Math.abs(-4));
        System.out.println("sys");
    "#,
        "2\nb\n[a, b]\n1\ntrue\nx1\n9\n4\nsys\n",
    );
}

#[test]
fn typed_variables_and_assignment() {
    check(
        r#"
        int i = 5;
        i += 2;
        print(i);
        long l = i;
        l *= 2;
        print(l);
        double d = 1.5;
        print((int) (d * 2));
        char c = 'a';
        print(c + 1);
        boolean b = !false;
        print(b);
        String s = 42;
        print(s);
    "#,
        "7\n14\n3\n98\ntrue\n42\n",
    );
}

#[test]
fn lambdas_and_closures() {
    check(
        r#"
        f = x -> x * 2;
        print(f(3));
        add = (a, b) -> a + b;
        print(add(2, 5));
        apply(fn, v) { return fn(v); }
        print(apply(x -> x + 1, 41));
        block = (n) -> { int t = 0; for (int i = 1; i <= n; i++) t += i; return t; };
        print(block(4));
        makeAdder(n) { return x -> x + n; }
        add5 = makeAdder(5);
        print(add5(3));
        total = 0;
        bump = () -> { total = total + 1; };
        bump(); bump();
        print(total);
    "#,
        "6\n7\n42\n10\n8\n2\n",
    );
}

#[test]
fn var_and_string_methods() {
    check(
        r#"
        var x = 1;
        var s = "ab";
        print(x + 1);
        print(s.repeat(2));
        print(" ".isBlank());
        print("a\nb".lines().count());
        java.util.function.Function f = (var v) -> v;
        print(f.apply("ok"));
        print("x%sy".formatted("Q"));
    "#,
        "2\nabab\ntrue\n2\nok\nxQy\n",
    );
}

#[test]
fn try_with_resources() {
    check(
        r#"
        try (java.io.StringReader r = new java.io.StringReader("z")) {
            print(r.read());
        }
        java.io.StringReader r2 = new java.io.StringReader("q");
        try (r2) { print(r2.read()); }
        try {
            r2.read();
            print("open");
        } catch (java.io.IOException e) {
            print("closed");
        }
    "#,
        "122\n113\nclosed\n",
    );
}

#[test]
fn try_with_resources_semantics() {
    check(
        r#"
        class R implements java.lang.AutoCloseable {
            String n;
            R(String n) { if (n == "bad") throw new RuntimeException("bad"); this.n = n; }
            public void close() { print("close-" + n); }
        }
        try (R a = new R("a"); R b = new R("b")) { print("body"); }
        try {
            try (R c = new R("c"); R d = new R("bad")) { print("never"); }
        } catch (java.lang.Throwable e) { print("caught-init"); }
        try {
            try (R e = new R("e")) { throw new RuntimeException("boom"); }
        } catch (java.lang.Throwable x) { print("caught-body"); }
        f() { try (R z = new R("z")) { return 7; } }
        print(f());
    "#,
        "body\nclose-b\nclose-a\nclose-c\ncaught-init\nclose-e\ncaught-body\nclose-z\n7\n",
    );
}

#[test]
fn semicolons_optional() {
    check(
        r#"
        var x = 1
        print(x + 1)
        class A {
            int v = 2
            int get() { return v }
        }
        print(new A().get())
    "#,
        "2\n2\n",
    );
}

#[test]
fn get_class_on_host_values() {
    check(
        r#"
        l = new java.util.ArrayList()
        l.add("a")
        print(l)
        print(l.getClass().getName())
        print("x".getClass().getName())
        print(new StringBuilder("s").getClass().getName())
    "#,
        "[a]\njava.util.ArrayList\njava.lang.String\njava.lang.StringBuilder\n",
    );
}

#[test]
fn reflection() {
    check(
        r#"
        import java.lang.reflect.*
        Class c = Class.forName("java.util.ArrayList")
        Object list = c.getConstructor().newInstance()
        Method add = c.getMethod("add", Class.forName("java.lang.Object"))
        add.invoke(list, "x")
        print(c.getMethod("size").invoke(list))
        Method[] methods = c.getDeclaredMethods()
        print(methods.length > 0)
        Class pc = Class.forName("java.awt.Point")
        Object p = pc.getConstructor(int.class, int.class).newInstance(3, 4)
        Field fx = pc.getField("x")
        fx.set(p, 42)
        print(fx.get(p))
    "#,
        "1\ntrue\n42\n",
    );
}

#[test]
fn assertions() {
    check(
        r#"
        assert(true);
        assertTrue(1 == 1);
        assertFalse(1 == 2);
        assertNull(null);
        assertNotNull("x");
        assertEquals(2, 2);
        assertNotEquals(2, 3);
        int[] a = { 1, 2, 3 };
        int[] b = { 1, 2, 3 };
        assertArrayEquals(a, b);
        print("assert-ok");
    "#,
        "assert-ok\n",
    );
}

#[test]
fn language_extras() {
    check(
        r#"
        import static java.lang.Math.max;
        import static java.lang.Integer.MAX_VALUE;
        assert 1 + 1 == 2;
        x = eval("2 + 3");
        print(x);
        this.interpreter.set("y", 7);
        print(this.interpreter.get("y"));
        print(max(4, 9));
        print(MAX_VALUE);
    "#,
        "5\n7\n9\n2147483647\n",
    );
}

#[test]
fn string_format() {
    check(
        r#"
        print(String.format("%02d|%.2f|%x|%s|%5s|%-5s|", 7, 1.234, 255, "ok", "a", "b"));
        print(String.format("%d%%", 50));
    "#,
        "07|1.23|ff|ok|    a|b    |\n50%\n",
    );
}

#[test]
fn method_references() {
    check(
        r#"
        abs = Math::abs;
        print(abs(-5));
        maximum = Math::max;
        print(maximum(2, 9));
        sb = new StringBuilder("x");
        append = sb::append;
        print(append("y").toString());
        print(String::valueOf(42));
        concat = "a"::concat;
        print(concat("b"));
    "#,
        "5\n9\nxy\n42\nab\n",
    );
}

#[test]
fn errors_are_reported() {
    let (_, result) = run("int x = 1; y = x + unknown;");
    assert!(result.is_err());
    let (_, result) = run("null.foo();");
    assert!(result.is_err());
}

#[test]
fn errors_are_catchable() {
    check(
        r#"
        try { nope(); } catch (e) { print("caught"); }
    "#,
        "caught\n",
    );
}

#[test]
fn modern_syntax_corpus() {
    use std::path::Path;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let files = [
        "13_var.bsh",
        "14_switch_arrow.bsh",
        "15_switch_expression.bsh",
        "16_instanceof_pattern.bsh",
        "17_record.bsh",
        "18_sealed.bsh",
        "19_unnamed.bsh",
        "20_string_templates.bsh",
        "21_switch_type_pattern.bsh",
        "22_record_pattern.bsh",
    ];
    for name in files {
        let src = std::fs::read_to_string(dir.join(name)).unwrap();
        let (_, result) = run(&src);
        assert!(result.is_ok(), "{}: {:?}", name, result.err());
    }
}

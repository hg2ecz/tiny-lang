# Mini Script Language (Rust Interpreter)

A tiny interpreted scripting language implemented in Rust.  
It’s intentionally small, but expressive enough for classic exercises like Fibonacci, prime search, and simple vector-based algorithms.

The language will feel familiar if you know **C** (statements, expressions, braces) and **Rust** (block scoping and a small “borrow-style” reference model for mutation).

---

## Quick Start

### 1) Build the interpreter

```bash
cargo build
```

### 2) Run a script

(Adjust the command below to match your project’s actual CLI if needed.)

```bash
cargo run -- path/to/program.lang
```
## Language Tour

### Values / Types

    Int (signed integer)

    Bool (true, false)

    String ("...")

    Vec (vector literal [a, b, c])

    Unit (like ())

### Variables

Use let to define variables, and = to assign.

```bash
let x = 10;
x = x + 5;
print x; // 15
```

### Strings

Strings are double-quoted. Common escapes are supported.

```bash
print "hello";
print "line1\nline2";
```

### Arithmetic (Int)

Operators: + - * / % and unary -.

```bash
let a = 7;
let b = 3;

print a + b; // 10
print a * b; // 21
print a % b; // 1
print -a;    // -7
```
Division/modulo by zero is a runtime error.

### Comparisons (Bool)

Operators: == != < <= > >=

```bash
let x = 10;
print x >= 5;   // true
print x == 7;   // false
```
Equality is supported for Int, Bool, and String.

### Control Flow

if / else

Condition must be Bool.

```bash
let n = 9;

if n % 2 == 0 {
    print "even";
} else {
    print "odd";
}
```
for ... in ... { ... }

for iterates over a vector value.
```bash
for x in [10, 20, 30] {
    print x;
}
```
### Functions

Define functions with fn. Use return to return a value.
```bash
fn add(a, b) {
    return a + b;
}

print add(2, 3); // 5
```
Recursion works:
```bash
fn fib(n) {
    if n <= 1 {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

print fib(10); // 55
```
## Vectors, Indexing, and References
### Vector literals and indexing
```bash
let xs = [5, 6, 7];
print xs[0]; // 5
```
Indexing produces a reference-like value internally (useful for mutation via set()).

### Borrow-style references: & and &mut

This language supports Rust-inspired references:

    &x produces an immutable reference

    &mut x produces a mutable reference

    also works with indexed elements: &xs[i], &mut xs[i]

The interpreter enforces simple Rust-like rules:

    you can’t mutate or move a value while it’s borrowed

    you can’t have a mutable borrow while immutable borrows exist (and vice versa)

These references are primarily used with built-in mutation functions.

## Built-in Functions

print expr;

Prints values. Strings print with quotes; vectors print like [ ... ].

len(x)

Length of a string or vector.
```bash
print len("abc");     // 3
print len([1,2,3,4]);  // 4

push(&mut vec, value)
```
Append to a vector.
```bash
let xs = [1, 2];
push(&mut xs, 3);
print xs; // [1, 2, 3]

set(&mut xs[i], value)
```
Mutate an element via a mutable element reference.
```bash
let xs = [10, 20, 30];
set(&mut xs[1], 99);
print xs; // [10, 99, 30]
```
range(end) / range(start, end)

Creates a vector of integers like Rust’s start..end (end-exclusive).
```bash
for i in range(5) {      // 0,1,2,3,4
    print i;
}

for i in range(2, 6) {   // 2,3,4,5
    print i;
}
```
## Example: Print primes up to 50
```bash
fn is_prime(n) {
    if n < 2 {
        return false;
    }

    for i in range(2, n) {
        if n % i == 0 {
            return false;
        }
    }

    return true;
}

for x in range(2, 51) {
    if is_prime(x) {
        print x;
    }
}
```
## Current Limitations

    No while, break, continue

    No logical operators (&&, ||, !) — use comparisons + nested ifs

    No floating-point type

    No structs/enums or pattern matching

    Vector/reference equality is not defined (only Int, Bool, String)

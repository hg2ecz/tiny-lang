// fib.t
fn fib(n) {
    if n <= 1 {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

let n = 10;
print fib(n);

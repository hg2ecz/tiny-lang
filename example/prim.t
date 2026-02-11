// primes.t
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

let limit = 50;

for x in range(2, limit + 1) {
    if is_prime(x) {
        print x;
    }
}

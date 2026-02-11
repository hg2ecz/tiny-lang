// demo_ownership.t

fn consume(v) {
    print "Inside consume():";
    print v;
    return 0;
}

let numbers = [1, 2, 3, 4];

print "Before calling consume():";
print &numbers;

consume(numbers);

print "FYI: here is an error, because 'numbers' is not owned.";
print &numbers;

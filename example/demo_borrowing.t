// borrow_demo.t
// Simple borrowing demonstration

// ------------------------------------------------------------
// Immutable borrow
// ------------------------------------------------------------

fn print_vector(v) {
    // v is expected to be an immutable reference (&vec)
    print "Inside print_vector():";
    print v;
    return 0;
}

let numbers = [10, 20, 30];

// Pass immutable reference
print_vector(&numbers);

// numbers is still usable
print "After immutable borrow:";
print numbers;

// ------------------------------------------------------------
// Mutable borrow
// ------------------------------------------------------------

fn add_value(v) {
    // v is expected to be &mut vec
    push(v, 99);
    return 0;
}

add_value(&mut numbers);

print "After mutable borrow:";
print numbers;


// ------------------------------------------------------------
// Borrow conflict demonstration
// ------------------------------------------------------------
print "ok"
// Create immutable reference to element
let first = numbers[0];
print first;

// While 'first' is alive, mutable borrow should fail.

// Uncomment to see runtime borrow error:
//
// add_value(&mut numbers);
//
// Error reason:
// Cannot mutably borrow `numbers`
// because it is already immutably borrowed.


// ------------------------------------------------------------
// Correct pattern
// ------------------------------------------------------------

// If you don't keep the reference alive,
// mutable borrow works again.

set(&mut numbers[0], 500);
print "After modification:";
print numbers;
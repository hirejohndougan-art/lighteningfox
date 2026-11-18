function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function apply(g, a) { return a.map(g).reduce((x, y) => x + y, 0); }
[fib(12), apply(fib, [1, 2, 3, 4]), apply((x) => x * x, [1, 2, 3])].join();

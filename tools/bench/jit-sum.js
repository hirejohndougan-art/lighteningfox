function sum(n) { let s = 0; for (let i = 0; i < n; i++) { s += i; } return s; }
function collatz(n) { let steps = 0; while (n !== 1) { if (n % 2 === 0) n = n / 2; else n = 3 * n + 1; steps++; } return steps; }
let t = 0;
for (let k = 0; k < 200; k++) t += sum(100000);
let c = 0;
for (let k = 1; k < 30000; k++) c += collatz(k);
print(t + " " + c);

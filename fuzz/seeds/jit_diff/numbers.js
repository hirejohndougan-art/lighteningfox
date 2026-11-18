function g(a, b) { let r = 0; for (let i = 0; i < 100; i++) { r = (r + a * i - b) % 1000; r = r | 0; } return [r, a / b, -0 * a, a % b]; }
String(g(3, 7)) + String(g(2147483647, -1)) + String(g(NaN, 0));

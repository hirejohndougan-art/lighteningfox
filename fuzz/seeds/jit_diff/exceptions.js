function t(x) { if (x % 3 === 0) throw new Error('e' + x); return x; }
function c(n) { let out = ''; for (let i = 0; i < n; i++) { try { out += t(i); } catch (e) { out += e.message; } finally { out += ';'; } } return out; }
c(8);

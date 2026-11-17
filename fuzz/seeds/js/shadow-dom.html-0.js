
const root = document.getElementById('js-host').attachShadow({ mode: 'open' });
root.innerHTML = '<style>:host{display:block;border:2px dashed #a50;padding:10px;background:#fff6e8} p{color:#a50;margin:0}</style><p>attachShadow() tree: <slot></slot></p>';
const fb = document.getElementById('fallback-host').attachShadow({ mode: 'closed' });
fb.innerHTML = '<style>:host{display:block;padding:8px;border:1px solid #999}</style><slot>Fallback content (nothing slotted)</slot>';
const r = [];
r.push('shadowRoot open: ' + (document.getElementById('js-host').shadowRoot === root));
r.push('closed hidden: ' + (document.getElementById('fallback-host').shadowRoot === null));
r.push('host: ' + (root.host.id));
r.push('mode: ' + root.mode);
r.push('declarative: ' + !!document.querySelector('.card').shadowRoot);
r.push('page query cannot see shadow: ' + (document.querySelector('.title') === null));
document.getElementById('report').textContent = r.join(' | ');

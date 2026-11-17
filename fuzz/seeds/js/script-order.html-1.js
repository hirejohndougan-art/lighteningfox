
function log(m) { var li = document.createElement('li'); li.textContent = m; document.getElementById('log').appendChild(li); }
log('1. inline script ran; footer present yet: ' + !!document.getElementById('footer') + ' (should be false)');

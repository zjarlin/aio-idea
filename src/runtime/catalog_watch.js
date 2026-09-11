await new Promise(resolve => {
  const done = () => {
    clearTimeout(timer);
    window.removeEventListener("focus", done);
    window.removeEventListener("aio:catalog-invalidated", done);
    resolve();
  };
  const timer = setTimeout(done, 30000);
  window.addEventListener("focus", done, { once: true });
  window.addEventListener("aio:catalog-invalidated", done, { once: true });
});
return true;

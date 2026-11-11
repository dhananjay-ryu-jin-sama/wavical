const { handle } = require('./wavical-backend.js');

handle('hello.greet', async (params) => {
  const name = (params && typeof params.name === 'string') ? params.name : 'World';
  return { message: `Hello, ${name}!` };
});


export default {
  appId: 'wavical-hello',
  window: {
    title: 'Wavical — Hello World',
    width: 1200,
    height: 800
  },
  capabilities: {
    'core.ping': true,
    'clipboard.read': true,
    'clipboard.write': true,
    'notify.send': true
  }
};

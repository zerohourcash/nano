import { defineConfig, devices } from '@playwright/test';

const token='playwright-local-token-32-characters';
export default defineConfig({
  testDir:'./e2e',
  timeout:30_000,
  fullyParallel:false,
  forbidOnly:true,
  retries:0,
  reporter:[['list'],['html',{open:'never'}]],
  use:{baseURL:'http://127.0.0.1:38900',trace:'retain-on-failure',screenshot:'only-on-failure',serviceWorkers:'block'},
  projects:[
    {name:'desktop',use:{...devices['Desktop Chrome'],viewport:{width:1440,height:1000}}},
    {name:'mobile',use:{...devices['Pixel 7']}},
  ],
  webServer:{
    command:'node src/server.js',url:'http://127.0.0.1:38900',reuseExistingServer:false,timeout:15_000,
    env:{...process.env,PORT:'38900',DATA_DIR:`.playwright-data/${process.pid}`,API_TOKEN:token,NODE_NAME:'E2E нода'},
  },
});

import {defineConfig} from '@playwright/test';
export default defineConfig({testDir:'./tests',timeout:30000,use:{baseURL:'http://127.0.0.1:1420',locale:'en-US',viewport:{width:1440,height:900},screenshot:'only-on-failure'},webServer:{command:'npm run dev',url:'http://127.0.0.1:1420',reuseExistingServer:!process.env.CI,env:{VITE_DEMO:'1'}}});

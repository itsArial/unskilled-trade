import {test,expect} from '@playwright/test';
test('desktop preview, wallet dossier, search, and mobile navigation',async({page})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 await page.setViewportSize({width:1440,height:1000});await page.goto('/');
 await expect(page.getByRole('heading',{name:'Find the signal.'})).toBeVisible();
 await expect(page.getByText('All wallet activity and performance below are synthetic.')).toBeVisible();
 await page.screenshot({path:'../docs/screenshots/desktop.png',fullPage:true,animations:'disabled'});
 await page.getByRole('button',{name:'Quiet conviction'}).click();
 await expect(page.getByRole('dialog',{name:'Wallet analysis'})).toBeVisible();
 await page.getByRole('button',{name:'Close wallet analysis'}).click();
 await page.getByRole('button',{name:'Smart wallets',exact:true}).click();
 await page.getByRole('textbox',{name:'Search wallets'}).fill('not-a-wallet');
 await expect(page.getByRole('heading',{name:'No wallets found'})).toBeVisible();
 await page.getByRole('button',{name:'Overview',exact:true}).click();
 await page.setViewportSize({width:390,height:844});
 await page.screenshot({path:'../docs/screenshots/mobile.png',fullPage:true,animations:'disabled'});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();
 await page.getByRole('button',{name:'Open navigation'}).click();
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Connect your edge.'})).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();
 expect(errors).toEqual([]);
});
test('administrator can import history, follow wallets, replay and manage plans',async({page})=>{
 await page.setViewportSize({width:1440,height:1000});await page.goto('/');
 await page.getByRole('button',{name:'Enter workspace'}).click();
 await page.getByRole('textbox',{name:'Email address'}).fill('admin@test.local');
 await page.getByLabel('Password',{exact:true}).fill('browser-test-password-123');
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click();
 await expect(page.getByRole('dialog',{name:'Sign in'})).not.toBeVisible();
 await page.getByRole('button',{name:'Administration',exact:true}).click();
 await page.getByRole('button',{name:'Load synthetic demo history'}).click();
 await expect(page.getByRole('status')).toContainText('360 synthetic demo events imported');

 await page.getByRole('button',{name:'Smart wallets',exact:true}).click();
 await expect(page.getByText('This database contains synthetic demo history.')).toBeVisible();
 await page.getByRole('button',{name:'Follow',exact:true}).first().click();
 await expect(page.getByRole('button',{name:'Following',exact:true})).toHaveCount(1);
 // The dossier must report follower outcomes and structural checks, not just
 // the leader's own profit.
 await page.locator('button.wallet-name').first().click();
 const dossier=page.getByRole('dialog',{name:'Wallet analysis'});
 await expect(dossier.getByRole('heading',{name:/^Copyability/})).toBeVisible();
 await expect(dossier.getByText('these results are optimistic.').first()).toBeVisible();
 // The leader's own profit and the follower's must be shown as different numbers.
 await expect(dossier.getByText(/own realized P&L was/)).toBeVisible();
 await expect(dossier.getByRole('cell',{name:'60s',exact:true})).toBeVisible();
 await expect(dossier.getByRole('heading',{name:'Authenticity',exact:true})).toBeVisible();
 await expect(dossier.getByText('Observed funders')).toBeVisible();
 await expect(dossier.getByText('unknown, not clean')).toBeVisible();
 await page.getByRole('button',{name:'Close wallet analysis'}).click();
 await expect(dossier).not.toBeVisible();
 await page.getByRole('button',{name:'Copy trading',exact:true}).click();
 await page.getByRole('button',{name:'Enable paper execution',exact:true}).click();
 await expect(page.getByRole('button',{name:'Pause execution',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Replay history',exact:true}).click();
 await expect(page.locator('tbody tr').first()).toBeVisible();
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 await page.getByRole('button',{name:'Create / rotate key'}).click();
 await expect(page.locator('.code-block').first()).toContainText('usk_');
 await page.getByRole('button',{name:'Revoke',exact:true}).click();
 await expect(page.locator('.code-block').first()).not.toContainText('usk_');
 await page.getByRole('button',{name:'Administration',exact:true}).click();
 await page.getByLabel('Plan ID',{exact:true}).fill('test-plan');
 await page.getByLabel('Display name',{exact:true}).fill('Test Plan');
 await page.getByLabel('Price in EUR',{exact:true}).fill('29');
 await page.getByRole('button',{name:'Save plan'}).click();
 await expect(page.getByRole('status')).toContainText('Plan saved');
 await page.getByRole('button',{name:'Subscription',exact:true}).click();
 await expect(page.getByText('TEST PLAN',{exact:true})).toBeVisible();

 // The funds widget shows a balance slot in the top bar and opens the detail.
 await page.getByRole('button',{name:'Your funds'}).click();
 const popover=page.getByRole('dialog',{name:'Funds'});
 await expect(popover.getByText('Commission owed')).toBeVisible();
 await expect(popover.getByText('only on profit above')).toBeVisible();
 await popover.getByRole('button',{name:'Deposit or withdraw'}).click();
 await expect(page.getByRole('heading',{name:'Deposit',exact:true})).toBeVisible();
 await expect(page.getByRole('heading',{name:'How we get paid'})).toBeVisible();
 // Custody is disclosed in plain words on the page that holds the money.
 await expect(page.getByText('That makes this service a custodian of your funds.')).toBeVisible();
 await expect(page.getByText('Withdrawals are not enabled on this server.')).toBeVisible();
});

test('wallet sign-in is offered and states that it authorizes nothing',async({page})=>{
 await page.goto('/');
 await page.getByRole('button',{name:'Enter workspace'}).click();
 const modal=page.getByRole('dialog',{name:'Sign in'});
 await expect(modal.getByRole('button',{name:'Continue with Phantom'})).toBeVisible();
 await expect(modal.getByRole('button',{name:'Continue with MetaMask'})).toBeVisible();
 await expect(modal.getByText('does not approve a transaction')).toBeVisible();
 // No wallet extension is installed in this browser, so it must say so rather
 // than fail silently.
 await modal.getByRole('button',{name:'Continue with Phantom'}).click();
 await expect(page.getByText('Phantom was not detected in this browser.').first()).toBeVisible();
});
test('registration gives an isolated account and paid features stay gated',async({page})=>{
 await page.goto('/');await page.getByRole('button',{name:'Enter workspace'}).click();
 await page.getByRole('button',{name:'Create an account',exact:true}).click();
 await page.getByRole('textbox',{name:'Email address'}).fill(`member-${Date.now()}@test.local`);
 await page.getByLabel('Password',{exact:true}).fill('browser-member-password-123');
 await page.getByRole('button',{name:'Create workspace',exact:true}).click();
 await expect(page.getByRole('dialog')).not.toBeVisible();
 await expect(page.getByRole('button',{name:'Administration',exact:true})).toHaveCount(0);
 await page.getByRole('button',{name:'Copy trading',exact:true}).click();
 await expect(page.getByRole('heading',{name:'A clean slate'})).toBeVisible();
 await page.getByRole('button',{name:'Enable paper execution',exact:true}).click();
 await expect(page.getByRole('alert')).toContainText('An active subscription is required');
});

test('a token page shows price, the account behind it, and refuses to fake what it does not know',async({page})=>{
 await page.route('**/api/discover',r=>r.fulfill({json:{profiled:[],observed:[
   {mint:'6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P',name:'Test Coin',symbol:'TEST',twitter:'https://x.com/testcoin',first_seen:1789700000}],
   note:'Neither list is complete.'}}));
 await page.route('**/api/market/**',r=>r.fulfill({json:{mint:'x',age_seconds:12,market:{
   pair_address:'POOL',dex:'pumpswap',name:'Test Coin',symbol:'TEST',price_usd:0.00042,price_native:0.0000021,
   liquidity_usd:1200,volume_h24:34000,market_cap:null,fdv:null,price_change_h24:-8.5,buys_h24:120,sells_h24:95,
   pair_created_at:1789700000,image_url:null,socials:[['twitter','https://x.com/testcoin']],websites:[],pools:2,
   warnings:['Only $1200 of liquidity: an ordinary order will move this price']},note:'Deepest pool only.'}}));
 // A plausible walk so the chart is exercised with real-shaped data.
 let px=0.00042; const series=Array.from({length:60},(_,i)=>{
   const o=px; px=px*(1+(Math.sin(i/3)+Math.cos(i/7))*0.03); const c=px;
   return {t:1789700000+i*3600,o,h:Math.max(o,c)*1.02,l:Math.min(o,c)*0.98,c,v:1000+i*37};});
 await page.route('**/candles**',r=>r.fulfill({json:{mint:'x',pool:'P',timeframe:'1h',age_seconds:8,
   candles:series,timeframes:['5m','1h','4h','1d'],note:'Deepest pool only.'}}));
 await page.route('**/api/tokens/**',r=>r.fulfill({json:{mint:'x',token:{},risk:null,last_price_sol:null,social:{
   handle:'testcoin',tokens_promoted:3,previous_mints:['AAA111','BBB222'],user_id:null,previous_handles:[],
   followers:null,account_created:null,identity_resolved:false,
   flags:['This account has fronted 2 other tokens we have seen','Account identity has not been resolved, so a rename could not be checked. Unknown, not clean.']}}}));
 await page.route('**/holders**',r=>r.fulfill({json:{mint:'x',supply:1000000,
   holders:[{address:'HolderAaa',amount:250000,share_pct:25},{address:'HolderBbb',amount:1000,share_pct:0.1}],
   top_traders:[{wallet:'TraderAaa',realized_pnl_sol:4.2,round_trips:6,round_trip_win_rate:66.6,score:71}],
   note:'Holders are token accounts, not people.'}}));


 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('/');
 await page.getByRole('button',{name:'Enter workspace'}).click();
 await page.getByRole('textbox',{name:'Email address'}).fill('admin@test.local');
 await page.getByLabel('Password',{exact:true}).fill('browser-test-password-123');
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click();
 await page.getByRole('button',{name:'Discover',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Every launch, weighed.'})).toBeVisible();
 await page.getByRole('button',{name:'Open',exact:true}).first().click();

 // Market figures, with their staleness stated rather than implied live.
 await expect(page.getByRole('heading',{name:/TEST/})).toBeVisible();
 await expect(page.getByText('$0.00042')).toBeVisible();
 await expect(page.getByText('as of 12s ago')).toBeVisible();
 // Market cap needs a price and a cap to derive supply; this fixture has no
 // cap, so the scale toggle must be unavailable rather than guessing.
 await expect(page.getByRole('button',{name:'MCap'})).toBeDisabled();
 await expect(page.getByText('an ordinary order will move this price')).toBeVisible();
 // Unreported market cap is a dash, never a zero.
 const stats=page.locator('.token-stats');
 await expect(stats.getByText('—').first()).toBeVisible();
 // The reuse signal, and the honest limit on it.
 await expect(page.getByText('fronted 2 other tokens')).toBeVisible();
 // Said in two places: the social panel and the safety panel. Both matter.
 await expect(page.getByText(/Unknown, not clean/)).toHaveCount(2);
 // No safety report collected must say so rather than imply safety.
 await expect(page.getByRole('heading',{name:'No safety report collected'})).toBeVisible();
 // Buying states who signs.
 await expect(page.getByText('never holds the key that signs a trade')).toBeVisible();

 // The chart: a legend is present because colour alone must not carry identity.
 const chart=page.locator('.candles');
 await expect(chart).toBeVisible();
 await expect(page.getByText('Close at or above open')).toBeVisible();
 await expect(page.getByText('Close below open')).toBeVisible();
 // Two rects per candle: the body and its volume bar beneath.
 expect(await page.locator('.candles rect').count()).toBe(120);
 // Volume shares the price frame rather than sitting in a second chart.
 await expect(page.locator('.candles').getByText('VOLUME')).toBeVisible();
 // Hover produces a readout rather than leaving people to guess values.
 await chart.hover({position:{x:400,y:150}});
 await expect(page.locator('.candle-tip')).toBeVisible();
 await expect(page.locator('.candle-tip').getByText('Volume')).toBeVisible();
 // A 1px vertical line has a zero-width box, so assert presence and position.
 await expect(page.locator('.candles .crosshair')).toHaveCount(1);
 const firstX=await page.locator('.candles .crosshair').getAttribute('x1');
 await chart.hover({position:{x:700,y:150}});
 expect(await page.locator('.candles .crosshair').getAttribute('x1')).not.toBe(firstX);
 // Holders are labelled as accounts, not people, and shares need a supply.
 await expect(page.getByRole('heading',{name:'Holders',exact:true})).toBeVisible();
 await expect(page.getByText('token accounts, not people')).toBeVisible();
 await expect(page.getByRole('heading',{name:'Traders we have seen here'})).toBeVisible();
 // Sell reads the real balance and offers nothing when there is none.
 await page.getByRole('button',{name:'Sell',exact:true}).click();
 await expect(page.getByText('You hold')).toBeVisible();
 await page.screenshot({path:'../docs/screenshots/token.png',fullPage:true,animations:'disabled'});
 expect(errors).toEqual([]);
});

test('a token with no market says so instead of showing a grid of dashes',async({page})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 await page.route('**/api/discover',r=>r.fulfill({json:{profiled:[
   {mint:'GTb96X8DM1mNyZfb3MrfRnYgs989dNF2fgEnXRvLbonk',icon:'',description:'A brand new coin'}],
   observed:[],note:'Neither list is complete.'}}));
 // Exactly what the providers return for a mint with no pool.
 await page.route('**/api/market/*',r=>r.fulfill({status:404,json:{error:'No market has been found for this mint. It may not be trading yet.'}}));
 await page.route('**/candles**',r=>r.fulfill({status:404,json:{error:'No pool was found for this mint, so there is nothing to chart.'}}));
 await page.route('**/holders**',r=>r.fulfill({status:502,json:{error:'RPC rejected the request'}}));
 await page.route('**/api/tokens/**',r=>r.fulfill({json:{mint:'x',token:null,social:null,risk:null,last_price_sol:null}}));

 await page.goto('/');
 await page.getByRole('button',{name:'Enter workspace'}).click();
 await page.getByRole('textbox',{name:'Email address'}).fill('admin@test.local');
 await page.getByLabel('Password',{exact:true}).fill('browser-test-password-123');
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click();
 await page.getByRole('button',{name:'Discover',exact:true}).click();
 await page.locator('.token-card').first().click();

 // The header uses what Discover already knew rather than "Unknown token".
 await expect(page.getByRole('heading',{name:/Not trading yet/})).toBeVisible();
 await expect(page.getByText('A brand new coin')).toBeVisible();
 // The empty state explains the cause and says it is the token's state.
 await expect(page.getByText(/no pool has been indexed/)).toBeVisible();
 await expect(page.getByText('That is the state of the token, not a failure to load.')).toBeVisible();
 await expect(page.getByRole('heading',{name:'Nothing to chart'})).toBeVisible();
 // Trading is disabled rather than offering a button that cannot work.
 // Two controls say "Buy": the side toggle and the submit. Only the submit
 // should be disabled; switching sides must still work.
 await expect(page.locator('.buy-panel .button.primary')).toBeDisabled();
 await expect(page.locator('.side-toggle button').first()).toBeEnabled();
 await expect(page.getByRole('button',{name:'Get price'})).toBeDisabled();
 await expect(page.getByText('no indexed market to route through')).toBeVisible();
 // A failing holders call must not blank the page.
 expect(errors).toEqual([]);
});

test('the top search finds coins and addresses, and paid-access chrome is gone under commission',async({page})=>{
 const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
 await page.route('**/api/discover',r=>r.fulfill({json:{profiled:[
   {mint:'6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P',market:{symbol:'MOVER',market_cap:120000,liquidity_usd:9000,price_change_h24:412.5,name:'Mover',price_usd:1,price_native:1,fdv:null,volume_h24:1,buys_h24:1,sells_h24:1,pair_created_at:null,image_url:null,socials:[],websites:[],pools:1,warnings:[],pair_address:'p',dex:'d'}},
   {mint:'pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn',market:null},
   {mint:'So11111111111111111111111111111111111111112',market:{symbol:'DEAD',market_cap:1,liquidity_usd:1,price_change_h24:null,name:'Dead',price_usd:1,price_native:1,fdv:null,volume_h24:0,buys_h24:0,sells_h24:0,pair_created_at:null,image_url:null,socials:[],websites:[],pools:1,warnings:[],pair_address:'p',dex:'d'}}],
   observed:[],note:'Neither list is complete.'}}));
 await page.route('**/api/search**',route=>{
  const q=new URL(route.request().url()).searchParams.get('q')||'';
  if(q.length>=32) return route.fallback();
  return route.fulfill({json:{exact:null,
    tokens:[{mint:'6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P',name:'Test Coin',symbol:'TEST'}],
    wallets:[],handles:['testcoin'],note:'Searches what this service has observed.'}});
 });
 // Only rewrite a successful identity: the logged-out 401 must pass through,
 // or the app believes a garbage user is signed in and never renders the door.
 await page.route('**/api/me',async route=>{
  const r=await route.fetch();
  if(!r.ok()) return route.fulfill({response:r});
  route.fulfill({json:{...await r.json(),access_model:'commission',copy_trading:false}});
 });
 await page.goto('/');
 await page.getByRole('button',{name:'Enter workspace'}).click();
 await page.getByRole('textbox',{name:'Email address'}).fill('admin@test.local');
 await page.getByLabel('Password',{exact:true}).fill('browser-test-password-123');
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click();

 // The ticker carries only tokens with a real market and a real move.
 await expect(page.locator('.ticker button').first()).toBeVisible();
 await expect(page.locator('.ticker').getByText('MOVER')).toHaveCount(2); // strip is duplicated for a seamless loop
 await expect(page.locator('.ticker').getByText('DEAD')).toHaveCount(0);
 // Each entry is a real control. Navigation is covered by the token-page
 // tests; asserting it here would only exercise the marquee's animation.
 await expect(page.locator('.ticker button').first()).toBeEnabled();

 // Signing in lands on the market, not on the logged-out explainer.
 await expect(page.getByRole('heading',{name:'Every launch, weighed.'})).toBeVisible();
 await page.setViewportSize({width:1440,height:1000});
 await page.screenshot({path:'../docs/screenshots/discover.png',animations:'disabled'});
 // No horizontal overflow: a max-content strip inside a flex row once pushed
 // the whole page sideways.
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();

 // The wordmark and the first nav item were four pixels apart, so an active
 // pill read as a button sitting inside the logo.
 const brand=await page.locator('.brand').boundingBox();
 const first=await page.locator('nav button').first().boundingBox();
 expect(first!.y-(brand!.y+brand!.height)).toBeGreaterThanOrEqual(20);

 // The search is a search bar, not a button parked next to the logo: it fills
 // the header and sits inside the bar's height. Its wrapper padding once
 // stacked on the global input padding and overflowed the bar.
 const bar=await page.locator('.global-search').boundingBox();
 const top=await page.locator('.topbar').boundingBox();
 expect(bar!.width).toBeGreaterThan(360);
 expect(bar!.height).toBeLessThanOrEqual(top!.height-8);
 expect(bar!.y).toBeGreaterThanOrEqual(top!.y);
 expect(bar!.y+bar!.height).toBeLessThanOrEqual(top!.y+top!.height);

 // The dead "Personal workspace" panel is gone.
 await expect(page.locator('.workspace')).toHaveCount(0);
 // Copy trading is off and access is free, so neither is advertised.
 await expect(page.getByRole('button',{name:'Copy trading',exact:true})).toHaveCount(0);
 await expect(page.getByRole('button',{name:'Smart wallets',exact:true})).toHaveCount(0);
 await expect(page.getByRole('button',{name:/Subscription/})).toHaveCount(0);

 // Typing searches; a pasted address offers to open directly.
 const box=page.getByRole('textbox',{name:'Search'});
 await box.fill('test');
 // "TEST" also appears inside the @testcoin handle, so match the token row.
 await expect(page.getByRole('option',{name:/^TEST Test Coin/})).toBeVisible();
 await expect(page.getByText('@testcoin')).toBeVisible();
 // This one goes to the real endpoint. Pasting an address blanked the whole
 // site because that branch returned no `handles` and the list called .map on
 // it; the app must survive the server's actual response, not a tidy mock.
 await box.fill('6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P');
 await expect(page.getByText('Open this token')).toBeVisible();
 await box.press('Enter');
 await expect(page.getByRole('heading',{name:'The whole picture.'})).toBeVisible();
 // Blank means the tree unmounted. The shell has to still be there.
 await expect(page.locator('.sidebar')).toBeVisible();
 await expect(page.locator('.crash')).toHaveCount(0);
 expect(errors).toEqual([]);
});

import {VersionedTransaction} from '@solana/web3.js';
import React, {useEffect, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Activity, ArrowDownLeft, ArrowUpRight, ArrowRight, ArrowLeft, Bell, Check, ChevronDown, ChevronRight, CircleHelp, Code2, Copy, CreditCard, Crosshair, Download, ExternalLink, Eye, Fingerprint, Layers3, LayoutDashboard, Link2, LogOut, Menu, MoreHorizontal, Pause, Play, Plus, Radio, Search, Settings2, Shield, SlidersHorizontal, Sparkles, Terminal, TrendingUp, Users, Wallet, X, Zap} from 'lucide-react';
import demoTrades from './demo-trades.json';
import '@fontsource/dm-sans/latin-400.css';
import '@fontsource/dm-sans/latin-500.css';
import '@fontsource/dm-sans/latin-600.css';
import '@fontsource/manrope/latin-400.css';
import '@fontsource/manrope/latin-500.css';
import '@fontsource/manrope/latin-600.css';
import '@fontsource/manrope/latin-700.css';
import './styles.css';

type Trade={id:string;wallet:string;token:string;symbol:string;side:string;quantity:number;price_sol:number;fee_sol:number;timestamp:number;liquidity_sol:number};
type Analysis={wallet:string;trades:number;matched_sells:number;win_rate:number|null;win_rate_interval_95?:[number,number]|null;expectancy_sol?:number|null;largest_win_share?:number|null;max_loss_streak?:number;synthetic?:boolean;realized_pnl_sol:number;profit_factor:number|null;max_drawdown_sol:number;median_hold_seconds:number|null;unmatched_quantity:number;score:number;flags:string[];curve:[number,number][];history:Trade[]};
type Position={id:string;wallet:string;symbol:string;cost:number;pnl:number|null;status:string;reason:string;created:number};
type Overview={wallets:Analysis[];feed:Trade[];watches:string[];positions:Position[];mode:string;source:string;last_event:number|null};
type Me={copy_trading?:boolean;access_model?:string;id:string;email:string;admin:boolean;subscribed:boolean;subscription_expires:number;enabled:boolean;budget:number};
type Plan={id:string;name:string;price_cents:number;days:number;active:number};
type Portfolio={address:string;sol_lamports:number|null;sol_error:string|null;tokens:{mint:string;amount:number;raw_amount:string;decimals:number;price_usd:number|null;value_usd:number|null}[];valued_usd:number;unpriced_tokens:number;holdings_error:string|null;note:string};
type Holders={mint:string;supply:number|null;holders:{address:string;amount:number;share_pct:number|null}[];top_traders:{wallet:string;realized_pnl_sol:number;round_trips:number;round_trip_win_rate:number|null;score:number}[];note:string};
type FeedData={events:Trade[];positions:{wallet:string;token:string;symbol:string;quantity:number;cost:number;entry:number;exit:number|null;pnl:number|null;status:string;reason:string;created:number;mark_price?:number|null;unrealized_pnl_sol?:number|null}[];note:string};
type Candle={t:number;o:number;h:number;l:number;c:number;v:number};
type CandleSet={mint:string;pool:string;timeframe:string;age_seconds:number;candles:Candle[];timeframes:string[];note:string};
type Market={pair_address:string|null;dex:string|null;name:string|null;symbol:string|null;price_usd:number|null;price_native:number|null;liquidity_usd:number|null;volume_h24:number|null;market_cap:number|null;fdv:number|null;price_change_h24:number|null;buys_h24:number|null;sells_h24:number|null;pair_created_at:number|null;image_url:string|null;socials:[string,string][];websites:string[];pools:number;warnings:string[]};
type SocialRep={handle:string;tokens_promoted:number;previous_mints:string[];user_id:string|null;previous_handles:string[];followers:number|null;account_created:number|null;identity_resolved:boolean;flags:string[]};
type TokenPage={mint:string;token:Record<string,unknown>|null;social:SocialRep|null;risk:{blocked:string|null;score:number|null;checked:number}|null;last_price_sol:number|null};
type QuoteSummary={in_amount:number;out_amount:number;minimum_out:number;price_impact_pct:number|null;platform_fee:number;platform_fee_bps:number;route:string[];warnings:string[]};
type SearchHit={exact:{address:string;kind:string}|null;tokens:{mint:string;name:string|null;symbol:string|null}[];wallets:string[];handles:string[];note:string};
type Discovered={profiled:{mint:string;icon?:string;description?:string;market:Market|null}[];observed:{mint:string;name:string|null;symbol:string|null;twitter:string|null;first_seen:number;creator?:string|null;copies?:number}[];note:string};
type Funds={address:string|null;custody_enabled:boolean;deposit_address:string|null;deposit_note:string;withdrawals_enabled:boolean;balance_lamports:number|null;balance_sol:number|null;balance_error:string|null;realized_pnl_sol:number;commission_owed_lamports:number;commission_charged_sol:number;withdrawable_lamports:number|null;reserved_lamports:number;fee_model:string;conflict_note:string;access_model:string;note:string};
type FeeRow={kind:string;lamports:number;basis_lamports:number;reference:string;created:number};
type FeeInfo={schedule:{model:string;performance_bps:number;trade_bps:number;epoch_seconds:number};conflict_note:string;cost_model:{fixed_lamports:number;bps:number;failure_rate:number};high_water_lamports:number;total_charged_lamports:number;ledger:FeeRow[]};
type Decay={delay_seconds:number;attempts:number;entered:number;resolved:number;unknown_liquidity:number;unfilled_entries:number;unresolved_exits:number;realized_pnl_sol:number;win_rate:number|null;expectancy_sol:number|null;median_entry_slippage_pct:number|null};
type Copyability={wallet:string;order_sol:number;cost_bps:number;leader_realized_pnl_sol:number;decay:Decay[];flags:string[]};
type Authenticity={wallet:string;buys:number;early_entries:number;early_entry_pct:number|null;early_window_seconds:number;funders:string[];co_funded_wallets:string[];funded_by_counterparty:string[];funding_graph_observed:boolean;flags:string[];blocked:string|null};
type Admin={users:{id:string;email:string;admin:number;banned:number;expires:number;enabled:number}[];logs:{id:number;user_id:string;action:string;detail:string;created:number}[];plans:Plan[];revenue_cents:number;payments:unknown[]};
const short=(s:string)=>s.slice(0,4)+'…'+s.slice(-4);
const number=(n:number,d=2)=>n.toLocaleString('en-US',{maximumFractionDigits:d,minimumFractionDigits:d});
const date=(n:number)=>new Date(n*1000).toLocaleDateString('en-US',{month:'short',day:'numeric'});
// Age of a market, in the coarsest unit that still says something. Seconds in,
// to match `pair_created_at`. A launch's age is the first thing a trader reads.
const age=(seconds:number|null|undefined)=>{if(!seconds)return null;
 const d=Math.floor(Date.now()/1000)-seconds; if(d<0)return null;
 if(d<3600)return Math.max(1,Math.floor(d/60))+'m';
 if(d<86400)return Math.floor(d/3600)+'h';
 return Math.floor(d/86400)+'d';};
const names=['Quiet conviction','The patient one','Early, not lucky','Against the grain','Second wave'];
// Four places to go, in the order the work actually happens: look, choose,
// copy, get paid. Signal feed lives inside Overview, and the two technical
// pages are one Settings page, because neither is a daily destination.
const NAV=[{id:'discover',name:'Discover',icon:Radio},{id:'overview',name:'Overview',icon:LayoutDashboard},{id:'wallets',name:'Smart wallets',icon:Wallet},{id:'execution',name:'Copy trading',icon:Layers3},{id:'portfolio',name:'Portfolio',icon:Activity},{id:'signals',name:'Feed',icon:Bell},{id:'funds',name:'Funds',icon:Wallet},{id:'settings',name:'Settings',icon:Code2}];
// Theme is a per-viewer preference, so it lives in the browser and is applied
// before first paint to avoid a flash of the wrong palette.
const THEMES=[{id:'void',name:'Void',hint:'Near-black violet'},{id:'obsidian',name:'Obsidian',hint:'Neutral near-black'},{id:'signal',name:'Signal',hint:'Original olive'}];
function storedTheme(){try{return localStorage.getItem('theme')||'void'}catch{return 'void'}}
function applyTheme(id:string){document.documentElement.dataset.theme=id;try{localStorage.setItem('theme',id)}catch{/* private mode */}}
applyTheme(storedTheme());
const B58='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
// Phantom returns raw signature bytes; the API expects base58.
function bs58(bytes:Uint8Array){let digits=[0];for(const byte of bytes){let carry=byte;for(let i=0;i<digits.length;i++){carry+=digits[i]<<8;digits[i]=carry%58;carry=(carry/58)|0}while(carry){digits.push(carry%58);carry=(carry/58)|0}}
 let out='';for(const b of bytes){if(b===0)out+='1';else break}
 return out+digits.reverse().map(d=>B58[d]).join('')}
async function api(path:string,method='GET',body?:unknown){const r=await fetch('/api'+path,{method,credentials:'same-origin',headers:body?{'Content-Type':'application/json'}:undefined,body:body?JSON.stringify(body):undefined});let v;try{v=await r.json()}catch{throw Error('The API is unavailable. Start the Rust backend.')}if(!r.ok)throw Error(v.error||'Request failed');return v;}
// The public preview is an explicitly synthetic dataset, never a live fallback.
function preview():Overview {const wallets=Array.from(new Set(demoTrades.map(t=>t.wallet))).map((wallet,i)=>{const history=demoTrades.filter(t=>t.wallet===wallet);let pnl=0,gains=0,losses=0,peak=0,dd=0,wins=0;const curve:[number,number][]=[];const buys=new Map<string,Trade>();history.forEach(t=>{if(t.side==='buy')buys.set(t.token,t);else{const b=buys.get(t.token)!;const p=t.quantity*(t.price_sol-b.price_sol)-t.fee_sol-b.fee_sol;pnl+=p;p>0?(gains+=p,wins++):losses-=p;peak=Math.max(peak,pnl);dd=Math.max(dd,peak-pnl);curve.push([t.timestamp,pnl]);}});return {wallet,history,trades:history.length,matched_sells:36,win_rate:wins/36*100,realized_pnl_sol:pnl,profit_factor:gains/losses,max_drawdown_sol:dd,median_hold_seconds:7600,unmatched_quantity:0,score:91-i*9,flags:['Synthetic preview data. Not a real wallet performance claim.','Score is a heuristic, not a probability of profit.'],curve};});return {wallets,feed:demoTrades.slice(-30).reverse(),watches:[],positions:[],mode:'preview',source:'Synthetic demonstration',last_event:demoTrades.at(-1)!.timestamp};}
const demo=preview();
// Candlesticks. Marks are thin, the grid is recessive, and the hover layer is
// standard rather than optional: an SVG chart people read prices off needs a
// crosshair and a per-candle readout.
// Money at memecoin scale: $1.2M reads, $1,240,000 does not.
function compact(n:number|null|undefined){if(n==null)return '—';const a=Math.abs(n);
 const [d,u]=a>=1e9?[n/1e9,'B']:a>=1e6?[n/1e6,'M']:a>=1e3?[n/1e3,'K']:[n,''];
 return '$'+d.toLocaleString(undefined,{maximumFractionDigits:a>=1e3?2:6})+u}
function pct(n:number|null|undefined){return n==null?'—':(n>0?'+':'')+n.toLocaleString(undefined,{maximumFractionDigits:2})+'%'}
function usd(n:number|null|undefined,digits=2){return n==null?'—':'$'+n.toLocaleString(undefined,{maximumFractionDigits:digits})}
function Candles({data,multiplier=1}:{data:Candle[];multiplier?:number}){
 const [hover,setHover]=useState<number|null>(null);
 const w=1000,h=300,padL=8,padR=62,padT=10,padB=22;
 // Volume shares the frame rather than taking a second chart: one time axis,
 // read together, which is how a price/volume pair is meant to be read.
 const volH=52,priceB=h-padB-volH;
 if(!data.length)return <Empty title="No candles yet" text="The provider returned no history for this pool."/>;
 const lows=Math.min(...data.map(c=>c.l*multiplier)),highs=Math.max(...data.map(c=>c.h*multiplier));
 const maxVol=Math.max(1,...data.map(c=>c.v));
 const span=highs-lows||highs||1;
 const y=(v:number)=>padT+(highs-v)/span*(priceB-padT);
 const step=(w-padL-padR)/data.length;
 const bodyW=Math.max(1,Math.min(9,step*0.62));
 const fmt=(v:number)=>multiplier!==1?compact(v):(v<0.01?v.toPrecision(3):number(v,4));
 const active=hover==null?null:data[hover];
 return <div className="chart-wrap">
  <svg className="candles" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img"
    aria-label={`Candlestick chart, ${data.length} periods, ${fmt(lows)} to ${fmt(highs)}`}
    onMouseLeave={()=>setHover(null)}
    onMouseMove={e=>{const r=(e.currentTarget as SVGSVGElement).getBoundingClientRect();
      const x=(e.clientX-r.left)/r.width*w;setHover(Math.max(0,Math.min(data.length-1,Math.floor((x-padL)/step))))}}>
   {[0,.25,.5,.75,1].map(f=>{const v=lows+span*f;return <g key={f}>
     <line className="grid" x1={padL} x2={w-padR} y1={y(v)} y2={y(v)}/>
     <text className="axis" x={w-padR+6} y={y(v)+3}>{fmt(v)}</text></g>})}
   {data.map((c,i)=>{const up=c.c>=c.o,x=padL+i*step+step/2;
     const [o,hi,lo,cl]=[c.o,c.h,c.l,c.c].map(v=>v*multiplier);
     const vh=Math.max(1,c.v/maxVol*(volH-6));
     return <g key={c.t} fill={up?'var(--up)':'var(--down)'} stroke={up?'var(--up)':'var(--down)'}>
      <line x1={x} x2={x} y1={y(hi)} y2={y(lo)} strokeWidth={1}/>
      <rect x={x-bodyW/2} y={Math.min(y(o),y(cl))} width={bodyW}
        height={Math.max(1,Math.abs(y(o)-y(cl)))} stroke="none"/>
      <rect x={x-bodyW/2} y={h-padB-vh} width={bodyW} height={vh} stroke="none" opacity={0.45}/></g>})}
   <line className="grid" x1={padL} x2={w-padR} y1={h-padB} y2={h-padB}/>
   {active&&<line className="crosshair" x1={padL+hover!*step+step/2} x2={padL+hover!*step+step/2} y1={padT} y2={h-padB}/>}
   <text className="axis" x={padL} y={h-padB-volH+10}>VOLUME</text>
   {[0,Math.floor(data.length/2),data.length-1].map(i=>data[i]&&
     <text key={i} className="axis" x={Math.min(w-padR-40,Math.max(padL,padL+i*step))} y={h-6}>{date(data[i].t)}</text>)}
  </svg>
  {active&&<div className="candle-tip" style={{left:`${Math.min(72,(hover!/data.length)*100)}%`,top:'8px'}}>
   <strong>{date(active.t)}</strong>
   <dl><dt>Open</dt><dd>{fmt(active.o*multiplier)}</dd><dt>High</dt><dd>{fmt(active.h*multiplier)}</dd>
   <dt>Low</dt><dd>{fmt(active.l*multiplier)}</dd><dt>Close</dt><dd>{fmt(active.c*multiplier)}</dd>
   <dt>Volume</dt><dd>{compact(active.v)}</dd></dl></div>}
 </div>}
function LineChart({values,small=false}:{values:number[];small?:boolean}){const id=React.useId().replaceAll(':','');const w=small?120:760,h=small?35:220;const min=Math.min(0,...values),max=Math.max(1,...values),pad=small?2:12;const coords=values.map((v,i)=>[pad+i*(w-pad*2)/Math.max(values.length-1,1),h-pad-(v-min)/(max-min)*(h-pad*2)]);const line=coords.map(p=>p.join(',')).join(' ');return <svg className={small?'sparkline':'line-chart'} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label="Cumulative realized profit in SOL"><defs><linearGradient id={id} x1="0" x2="0" y1="0" y2="1"><stop stopColor="var(--accent)" stopOpacity=".16"/><stop offset="1" stopColor="var(--accent)" stopOpacity="0"/></linearGradient></defs>{!small&&[0,1,2,3,4].map(i=><line key={i} x1="0" x2={w} y1={i*h/4} y2={i*h/4} stroke="var(--sheen)" strokeDasharray="3 5"/>)}{coords.length>0&&<><polygon points={`${pad},${h} ${line} ${w-pad},${h}`} fill={`url(#${id})`}/><polyline points={line} fill="none" stroke="var(--accent)" strokeWidth={small?1.5:2} strokeLinejoin="round" vectorEffect="non-scaling-stroke"/>{!small&&<circle cx={coords.at(-1)![0]} cy={coords.at(-1)![1]} r="4" fill="var(--accent)"/>}</>}</svg>}
function App(){const [me,setMe]=useState<Me|null>(null),[data,setData]=useState<Overview>(demo),[page,setPage]=useState('overview'),[query,setQuery]=useState(''),[authOpen,setAuthOpen]=useState(false),[register,setRegister]=useState(false),[error,setError]=useState(''),[toast,setToast]=useState(''),[busy,setBusy]=useState(false),[selected,setSelected]=useState<Analysis|null>(null),[menu,setMenu]=useState(false),[period,setPeriod]=useState('30D'),[feedFilter,setFeedFilter]=useState('All activity'),[plans,setPlans]=useState<Plan[]>([]),[admin,setAdmin]=useState<Admin|null>(null),[apiKey,setApiKey]=useState(''),[budget,setBudget]=useState(.1),[sort,setSort]=useState('score'),[rpcResult,setRpcResult]=useState<{scanned:number;archived:number;decoded:number;inserted:number;next_before:string|null;coverage:string}|null>(null),[funds,setFunds]=useState<Funds|null>(null),[feeInfo,setFeeInfo]=useState<FeeInfo|null>(null),[fundsOpen,setFundsOpen]=useState(false),[theme,setTheme]=useState(storedTheme()),[discovered,setDiscovered]=useState<Discovered|null>(null),[mint,setMint]=useState(''),[market,setMarket]=useState<Market|null>(null),[marketAge,setMarketAge]=useState(0),[tokenPage,setTokenPage]=useState<TokenPage|null>(null),[quote,setQuote]=useState<QuoteSummary|null>(null),[buySol,setBuySol]=useState(0.1),[slippage,setSlippage]=useState(100),[candles,setCandles]=useState<CandleSet|null>(null),[tf,setTf]=useState('1h'),[scale,setScale]=useState<'price'|'mcap'>('price'),[sortBy,setSortBy]=useState('mcap'),[term,setTerm]=useState(''),[results,setResults]=useState<SearchHit|null>(null),[side,setSide]=useState<'buy'|'sell'>('buy'),[sellPct,setSellPct]=useState(100),[portfolio,setPortfolio]=useState<Portfolio|null>(null),[holders,setHolders]=useState<Holders|null>(null),[hint,setHint]=useState<{name?:string;icon?:string;description?:string}|null>(null),[socialFeed,setSocialFeed]=useState<FeedData|null>(null),[released,setReleased]=useState<{address:string;secret_key:string;warning:string}|null>(null),[copy,setCopy]=useState<Copyability|null>(null),[authentic,setAuthentic]=useState<Authenticity|null>(null);
const logged=!!me;
// Copy trading is a separate product the operator can switch off. When it is
// off the entry disappears rather than leading to a page that refuses.
// Only what is live: the copy-trading pages vanish with the feature, and the
// research preview is for people who have not signed in yet.
// Overview, Smart wallets and Copy trading are one product: the research
// dashboard exists to serve copying. They appear and vanish together, and
// Overview is also the signed-out explainer, so it stays for visitors.
const nav=NAV.filter(n=>
 !['execution','wallets','overview'].includes(n.id)
 ||me?.copy_trading!==false
 ||(!logged&&n.id==='overview'));
async function refresh(){const u=await api('/me');setMe(u);setBudget(u.budget);setData(await api('/overview'));if(u.admin)setAdmin(await api('/admin'));await loadFunds();}
async function loadFunds(){try{setFunds(await api('/funds'));setFeeInfo(await api('/fees'))}catch{/* the widget simply stays empty */}}
useEffect(()=>{api('/me').then(async u=>{setMe(u);setBudget(u.budget);setData(await api('/overview'));if(u.admin)setAdmin(await api('/admin'));await loadFunds();}).catch(()=>{});api('/plans').then(setPlans).catch(()=>{});},[]);
useEffect(()=>{applyTheme(theme)},[theme]);
useEffect(()=>{if(logged&&page==='overview')setPage('discover')},[logged]);
useEffect(()=>{if(logged&&!discovered)loadDiscover()},[logged]);
useEffect(()=>{if(page==='portfolio'&&logged&&!portfolio)run(async()=>setPortfolio(await api('/portfolio')))},[page,logged]);
useEffect(()=>{if(page==='signals'&&logged)run(async()=>setSocialFeed(await api('/feed')))},[page,logged]);
useEffect(()=>{if(toast){const t=setTimeout(()=>setToast(''),4500);return()=>clearTimeout(t)}},[toast]);
useEffect(()=>{const esc=(e:KeyboardEvent)=>{if(e.key==='Escape'){setSelected(null);setAuthOpen(false);setMenu(false)}};window.addEventListener('keydown',esc);return()=>window.removeEventListener('keydown',esc)},[]);
async function run(fn:()=>Promise<void>){setBusy(true);setError('');try{await fn()}catch(e){setError((e as Error).message)}finally{setBusy(false)}}
// Wallet sign-in proves control of an address. It authorizes nothing else, and
// the message the wallet displays says so.
async function walletSignIn(kind:'phantom'|'metamask'){await run(async()=>{
 const w=window as unknown as {solana?:{isPhantom?:boolean;connect:()=>Promise<{publicKey:{toString:()=>string}}>;signMessage:(m:Uint8Array,e:string)=>Promise<{signature:Uint8Array}>};ethereum?:{request:(a:{method:string;params?:unknown[]})=>Promise<unknown>}};
 if(kind==='phantom'){
  if(!w.solana?.isPhantom)throw Error('Phantom was not detected in this browser.');
  const {publicKey}=await w.solana.connect();const address=publicKey.toString();
  const c=await api('/auth/wallet/challenge','POST',{address,chain:'solana'});
  const {signature}=await w.solana.signMessage(new TextEncoder().encode(c.message),'utf8');
  await api('/auth/wallet/verify','POST',{address,chain:'solana',nonce:c.nonce,signature:bs58(signature)});
 }else{
  if(!w.ethereum)throw Error('MetaMask was not detected in this browser.');
  const accounts=await w.ethereum.request({method:'eth_requestAccounts'}) as string[];
  const address=accounts[0];
  const c=await api('/auth/wallet/challenge','POST',{address,chain:'ethereum'});
  const signature=await w.ethereum.request({method:'personal_sign',params:[c.message,address]}) as string;
  await api('/auth/wallet/verify','POST',{address,chain:'ethereum',nonce:c.nonce,signature});
 }
 await refresh();setAuthOpen(false);setToast('Signed in with your wallet')})}
const SOL_MINT='So11111111111111111111111111111111111111112';
const lamports=(sol:number)=>Math.round(sol*1e9);
// A token page is only meaningful with a market; every figure states its age.
async function openToken(target:string,known?:{name?:string;icon?:string;description?:string}){
 setMint(target);setPage('token');setQuote(null);setMarket(null);setTokenPage(null);setHint(known||null);
 await run(async()=>{
  const [m,t]=await Promise.all([
   api('/market/'+encodeURIComponent(target)).catch(()=>null),
   api('/tokens/'+encodeURIComponent(target)).catch(()=>null)]);
  if(m){setMarket(m.market);setMarketAge(m.age_seconds)}
  if(t)setTokenPage(t);
  setCandles(await api(`/market/${encodeURIComponent(target)}/candles?tf=${tf}`).catch(()=>null));
  setHolders(await api(`/tokens/${encodeURIComponent(target)}/holders`).catch(()=>null));
 })}
async function loadCandles(next:string){setTf(next);if(!mint)return;
 await run(async()=>setCandles(await api(`/market/${encodeURIComponent(mint)}/candles?tf=${next}`).catch(()=>null)))}
// Supply implied by the market: cap divided by price. Absent when either is
// unreported, which disables the market-cap scale rather than inventing one.
function capMultiplier(){const c=market?.market_cap,p=market?.price_usd;
 return c&&p&&p>0?c/p:null}
// Searching is cheap and local, so it runs as you type. A pasted address
// short-circuits: somebody who pasted 44 characters already knows the answer.
let searchSeq=0;
async function runSearch(q:string){
 const mine=++searchSeq;
 if(q.trim().length<2){setResults(null);return}
 try{const r=await api('/search?q='+encodeURIComponent(q.trim()));if(mine===searchSeq)setResults(r)}
 catch{if(mine===searchSeq)setResults(null)}}
function jumpTo(){
 const q=term.trim();
 if(results?.exact){setTerm('');setResults(null);
  if(results.exact.kind==='wallet')detail({wallet:results.exact.address} as Analysis);
  else openToken(results.exact.address);
  return}
 const first=results?.tokens[0];
 if(first){setTerm('');setResults(null);openToken(first.mint,{name:first.symbol||undefined})}
 else if(q)setToast('Nothing matched. Paste a mint address to open a token we have not seen.')}
// The strip only carries tokens with a real market and a real move; padding
// it with zeroes would make a dead market look busy.
const tickerItems=(discovered?.profiled||[])
 .filter(t=>t.market&&t.market.price_change_h24!=null)
 .sort((a,b)=>Math.abs(b.market!.price_change_h24||0)-Math.abs(a.market!.price_change_h24||0))
 .slice(0,14);
async function loadDiscover(){await run(async()=>setDiscovered(await api('/discover')))}
// What this account holds of the open token, from the on-chain portfolio.
function heldAmount(){const t=portfolio?.tokens.find(t=>t.mint===mint);return t?{raw:BigInt(t.raw_amount),amount:t.amount,decimals:t.decimals}:null}
function sellRaw(){const h=heldAmount();return h?(h.raw*BigInt(Math.round(sellPct))/100n):0n}
function swapLegs(){return side==='buy'
 ? {input_mint:SOL_MINT,output_mint:mint,amount:lamports(buySol)}
 : {input_mint:mint,output_mint:SOL_MINT,amount:Number(sellRaw())}}
async function priceSwap(){if(!needAuth())return;await run(async()=>{
 const legs=swapLegs();
 if(!legs.amount)throw Error(side==='sell'?'You hold none of this token, or the portfolio has not loaded.':'Enter an amount.');
 setQuote((await api('/swap/quote','POST',{...legs,slippage_bps:slippage})).quote)})}
// The server builds the transaction; the wallet signs it. We never see the key.
async function executeSwap(){if(!needAuth())return;await run(async()=>{
 const w=window as unknown as {solana?:{isPhantom?:boolean;publicKey?:{toString:()=>string};connect:()=>Promise<unknown>;signAndSendTransaction:(t:VersionedTransaction)=>Promise<{signature:string}>}};
 if(!w.solana?.isPhantom)throw Error('Phantom was not detected. Trading needs a wallet that can sign.');
 if(!w.solana.publicKey)await w.solana.connect();
 const taker=w.solana.publicKey?.toString();
 const legs=swapLegs();
 if(!legs.amount)throw Error(side==='sell'?'You hold none of this token.':'Enter an amount.');
 const built=await api('/swap/build','POST',{...legs,slippage_bps:slippage,taker});
 const bytes=Uint8Array.from(atob(built.transaction),c=>c.charCodeAt(0));
 try{
  const {signature}=await w.solana.signAndSendTransaction(VersionedTransaction.deserialize(bytes));
  await api('/swap/submitted','POST',{id:built.id,signature,submitted:true});
  setPortfolio(await api('/portfolio').catch(()=>portfolio));
  setToast('Submitted: '+signature.slice(0,16)+'… confirm it on chain before treating it as final');
 }catch(e){
  // A refusal in the wallet is a real outcome, recorded as abandoned.
  await api('/swap/submitted','POST',{id:built.id,submitted:false}).catch(()=>{});
  throw e;
 }})}
function needAuth(){if(!me){setAuthOpen(true);return false}return true}
async function follow(w:string){if(!needAuth())return;await run(async()=>{const followed=data.watches.includes(w);await api('/watches'+(followed?'/'+encodeURIComponent(w):''),followed?'DELETE':'POST',followed?undefined:{wallet:w});await refresh();setToast(followed?'Wallet removed from watchlist':'Wallet added to watchlist')})}
function navigate(p:string){setPage(p);setMenu(false);setError('');setQuery('')}
async function detail(w:Analysis){setCopy(null);setAuthentic(null);if(!me){setSelected(w);return}await run(async()=>{const path=encodeURIComponent(w.wallet);setSelected(await api('/wallets/'+path));const [c,a]=await Promise.all([api('/wallets/'+path+'/copyability').catch(()=>null),api('/wallets/'+path+'/authenticity').catch(()=>null)]);setCopy(c);setAuthentic(a)})}
const filtered=data.wallets.filter(w=>w.wallet.toLowerCase().includes(query.toLowerCase())||names[data.wallets.indexOf(w)]?.toLowerCase().includes(query.toLowerCase())).sort((a,b)=>sort==='pnl'?b.realized_pnl_sol-a.realized_pnl_sol:b.score-a.score);
const leader=data.wallets[0];const cutoff=(data.last_event||0)-(period==='7D'?7:period==='30D'?30:365)*86400;
const curve=leader?.curve.filter(v=>v[0]>=cutoff)||[];
const aggregate=data.wallets.reduce((s,w)=>s+w.realized_pnl_sol,0);
const feed=data.feed.filter(t=>(feedFilter==='All activity'||t.side===(feedFilter==='Buys'?'buy':'sell'))&&(t.symbol.toLowerCase().includes(query.toLowerCase())||t.wallet.toLowerCase().includes(query.toLowerCase())));
const pnl=data.positions.reduce((s,p)=>s+(p.pnl||0),0);
const SectionHeading=({title,sub,children}:{title:string;sub:string;children?:React.ReactNode})=><div className="section-heading"><div><h2>{title}</h2><p>{sub}</p></div>{children}</div>;
function WalletTable({compact=false}:{compact?:boolean}){return <div className="table-scroll"><table className="wallet-table"><thead><tr><th>Wallet</th><th>Realized P&L <span>↓</span></th><th>Win rate</th><th>Score <CircleHelp size={12}/></th><th>Performance</th><th></th></tr></thead><tbody>{filtered.slice(0,compact?5:100).map((w,i)=><tr key={w.wallet}><td><button className="wallet-name" onClick={()=>detail(w)}><span className={`avatar avatar-${i%5}`}><Fingerprint size={20}/></span><span><strong>{logged?short(w.wallet):names[i]||short(w.wallet)}</strong><small>{short(w.wallet)} <Copy size={10}/></small></span>{i===0&&<span className="tiny-label">TOP</span>}</button></td><td className={w.realized_pnl_sol>=0?'positive':'negative'}>{w.realized_pnl_sol>=0?'+':''}{number(w.realized_pnl_sol)} <small>SOL</small></td><td><span>{w.win_rate===null?'—':number(w.win_rate,1)+'%'}</span><div className="mini-track"><i style={{width:(w.win_rate||0)+'%'}}/></div></td><td><span className={'score '+(w.score>=75?'high':'medium')}>{w.score}<small>/100</small></span></td><td><LineChart small values={w.curve.map(p=>p[1])}/></td><td><button className={'follow-button '+(data.watches.includes(w.wallet)?'following':'')} onClick={()=>follow(w.wallet)} disabled={busy}>{data.watches.includes(w.wallet)?<Check size={13}/>:<Plus size={13}/>} {data.watches.includes(w.wallet)?'Following':'Follow'}</button></td></tr>)}</tbody></table>{!filtered.length&&<Empty title="No wallets found" text={query?'Try a different address.':'Import trade history in Administration to start analyzing wallets.'}/>}</div>}
function FeedList({full=false}:{full?:boolean}){return <div className="feed-list">{feed.slice(0,full?40:5).map(t=><div className="feed-row" key={t.id}><span className={'trade-icon '+t.side}>{t.side==='buy'?<ArrowDownLeft size={17}/>:<ArrowUpRight size={17}/>}</span><div className="feed-main"><strong>{short(t.wallet)} <span>{t.side==='buy'?'bought':'sold'}</span> {t.symbol}</strong><small>{date(t.timestamp)} · {number(t.quantity*t.price_sol)} SOL</small></div><span className={'tag '+(t.side==='buy'?'green':'muted')}>{t.side==='buy'?'ENTRY':'EXIT'}</span></div>)}{!feed.length&&<Empty title="No activity yet" text="Imported wallet events will appear here."/>}</div>}
return <div className="app-shell">
<div className="main-shell">
{logged&&!!tickerItems.length&&<div className="ticker" aria-label="Trending tokens">
 <div className="ticker-track">{[0,1].map(copy=><div className="ticker-run" key={copy} aria-hidden={copy===1}>
  {tickerItems.map(t=><button key={copy+t.mint} onClick={()=>openToken(t.mint,{name:t.market!.symbol||undefined})}>
   <span className="ticker-name">{t.market!.symbol||short(t.mint)}</span>
   <span className="ticker-cap">{compact(t.market!.market_cap??t.market!.liquidity_usd)}</span>
   <span className={(t.market!.price_change_h24||0)>=0?'positive':'negative'}>{pct(t.market!.price_change_h24)}</span>
  </button>)}</div>)}</div></div>}<aside className={'sidebar '+(menu?'mobile-open':'')}><a className="brand" href="#" onClick={e=>{e.preventDefault();navigate('overview')}}><span className="brand-mark"><i/><i/><i/></span>unskilled<span className="brand-dot">®</span></a><nav>{nav.map(n=><button key={n.id} className={page===n.id?'active':''} onClick={()=>navigate(n.id)}><n.icon size={18}/>{n.name}{n.id==='signals'&&<span className="nav-count">{data.feed.length}</span>}{page===n.id&&<i/>}</button>)}</nav><div className="sidebar-bottom"><div className="sidebar-note"><span className="status-dot"/> {logged?'Local analysis engine':'Explore the workspace'}<p>{logged?'Paper execution · No live orders':'Synthetic data · No live orders'}</p></div>{me?.access_model!=='commission'&&<button className={page==='billing'?'active':''} onClick={()=>navigate('billing')}><CreditCard size={17}/>Subscription<ArrowUpRight size={14}/></button>}{me?.admin&&<button className={page==='admin'?'active':''} onClick={()=>navigate('admin')}><Shield size={17}/>Administration</button>}<button onClick={()=>navigate('research')}><CircleHelp size={17}/>Research & methodology</button><div className="profile"><span className="profile-avatar">{me?me.email[0].toUpperCase():'G'}</span><div><strong>{me?me.email.split('@')[0]:'Guest explorer'}</strong><small>{me?.admin?'Administrator':me?.subscribed?'Active subscription':'Preview workspace'}</small></div><button title={me?'Sign out':'Sign in'} aria-label={me?'Sign out':'Sign in'} onClick={()=>me?run(async()=>{await api('/auth/logout','POST');setMe(null);setData(demo);setAdmin(null);navigate('overview')}):setAuthOpen(true)}>{me?<LogOut size={16}/>:<ArrowRight size={16}/>}</button></div></div></aside>{menu&&<div className="sidebar-scrim" onClick={()=>setMenu(false)}/>}<header className="topbar"><div className="breadcrumbs"><button className="mobile-menu icon-button" onClick={()=>setMenu(!menu)} aria-label="Open navigation"><Menu size={20}/></button>
{logged?<div className="global-search"><Search size={15}/>
<input value={term} onChange={e=>{setTerm(e.target.value);runSearch(e.target.value)}} onKeyDown={e=>{if(e.key==='Enter')jumpTo()}} placeholder="Search coins, wallets, or paste an address" aria-label="Search" maxLength={128}/>
{!!term&&<button className="icon-button" aria-label="Clear search" onClick={()=>{setTerm('');setResults(null)}}><X size={15}/></button>}
{results&&<div className="search-results" role="listbox">
{results.exact&&<button role="option" aria-selected="false" onClick={()=>jumpTo()}><strong>Open this {results.exact.kind}</strong><small>{results.exact.address}</small></button>}
{(results.tokens||[]).map(t=><button key={t.mint} role="option" aria-selected="false" onClick={()=>{setTerm('');setResults(null);openToken(t.mint,{name:t.symbol||undefined})}}><strong>{t.symbol||'Token'} <span className="subtle-label">{t.name||''}</span></strong><small>{short(t.mint)}</small></button>)}
{(results.wallets||[]).map(w=><button key={w} role="option" aria-selected="false" onClick={()=>{setTerm('');setResults(null);detail({wallet:w} as Analysis)}}><strong>Wallet</strong><small>{short(w)}</small></button>)}
{(results.handles||[]).map(x=><button key={x} role="option" aria-selected="false" onClick={()=>{setTerm('');setResults(null);navigate('discover')}}><strong>@{x}</strong><small>X account seen on a token</small></button>)}
{!results.exact&&!results.tokens?.length&&!results.wallets?.length&&!results.handles?.length&&<div className="search-empty">Nothing matched. {results.note}</div>}
</div>}</div>:<><span>Workspace</span><ChevronRight size={12}/></>}
{!logged&&<strong>{nav.find(n=>n.id===page)?.name||({billing:'Subscription',admin:'Administration',research:'Research & methodology',signals:'Feed',token:'Token',portfolio:'Portfolio'} as Record<string,string>)[page]}</strong>}</div><div className="topbar-right"><span className="chain"><span className="solana-mark">≋</span> Solana</span><span className="topbar-divider"/><div className="theme-switch" role="group" aria-label="Colour theme">{THEMES.map(t=><button key={t.id} className={theme===t.id?'selected':''} aria-pressed={theme===t.id} title={t.hint} onClick={()=>setTheme(t.id)}>{t.name}</button>)}</div><span className="topbar-divider"/><button className="mode-pill" onClick={()=>navigate('execution')}><span className="status-dot"/>{logged?'Paper mode':'Preview mode'}</button><button className="icon-button" aria-label="View activity" onClick={()=>navigate('signals')}><Bell size={17}/></button>{logged&&<button className="funds-pill" onClick={()=>{setFundsOpen(!fundsOpen);loadFunds()}} aria-label="Your funds"><Wallet size={14}/><strong>{funds?.balance_sol==null?'—':number(funds.balance_sol,3)}</strong><small>SOL</small></button>}<span className="top-avatar" onClick={()=>!me&&setAuthOpen(true)}>{me?me.email[0].toUpperCase():'G'}</span>{fundsOpen&&logged&&<div className="funds-popover" role="dialog" aria-label="Funds">
<div className="funds-top"><span className="overline">YOUR WALLET</span><button className="icon-button" aria-label="Close funds" onClick={()=>setFundsOpen(false)}><X size={16}/></button></div>
<strong className="funds-balance">{funds?.balance_sol==null?'Unavailable':number(funds.balance_sol,4)+' SOL'}</strong>
{funds?.balance_error&&<p className="fine-print">Balance could not be read right now: {funds.balance_error}. This is not a zero balance.</p>}
<div className="rules"><span>Realized P&amp;L<strong className={(funds?.realized_pnl_sol||0)>=0?'positive':'negative'}>{number(funds?.realized_pnl_sol||0)} SOL</strong></span>
<span>Commission owed<strong>{number((funds?.commission_owed_lamports||0)/1e9,4)} SOL</strong></span>
<span>Withdrawable<strong>{funds?.withdrawable_lamports==null?'—':number(funds.withdrawable_lamports/1e9,4)+' SOL'}</strong></span></div>
{funds?.address&&<button className="address-copy" onClick={()=>run(async()=>{await navigator.clipboard.writeText(funds.address!);setToast('Deposit address copied')})}>{funds.address}<Copy size={13}/></button>}
<p className="fine-print">{funds?.conflict_note}</p>
<button className="button" onClick={()=>{setFundsOpen(false);navigate('funds')}}>Deposit or withdraw <ArrowRight size={14}/></button></div>}</div></header>
<main><div className={'page-heading'+(page==='discover'?' compact':'')}><div><div className="eyebrow"><span/> ONCHAIN INTELLIGENCE</div><h1>{({overview:'Find the signal.',wallets:'Follow the evidence.',signals:'Every move tells a story.',execution:'Your rules. Every trade.',settings:'Connect your edge.',funds:'Your money, in plain sight.',discover:'Every launch, weighed.',token:'The whole picture.',portfolio:'What you actually hold.',billing:'A plan for your process.',admin:'Behind the terminal.',research:'Know what you’re following.'} as Record<string,string>)[page]}</h1><p>{({overview:'Understand the wallets. See the conviction. Move with context.',wallets:'Look beyond a winning trade. Understand the full track record.',signals:'A chronological view of the wallet activity in your dataset.',execution:'Test a strategy before you trust it with capital.',settings:'Data access, trading permission and API keys, clearly separated.',funds:'Deposit, withdraw, and see exactly what a trade costs you.',discover:'New markets, with what is known and what is not.',token:'Price, safety and the account behind it, side by side.',portfolio:'Read from the chain, priced where a price exists.',billing:'Simple subscriptions. Clear limits. No promises of returns.',admin:'Manage access, inspect activity, and keep the operation accountable.',research:'Transparent assumptions make better decisions possible.'} as Record<string,string>)[page]}</p></div><div className="heading-actions">{!me?<button className="button primary" onClick={()=>setAuthOpen(true)}>Enter workspace <ArrowUpRight size={16}/></button>:<button className="button" onClick={()=>run(refresh)} disabled={busy}><Activity size={15}/> Refresh data</button>}</div></div>
{error&&<div className="notice error" role="alert">{error}<button aria-label="Dismiss error" onClick={()=>setError('')}><X size={16}/></button></div>}
{!logged&&<div className="preview-banner"><span><Eye size={15}/><strong>You’re exploring a sample workspace.</strong><span> All wallet activity and performance below are synthetic.</span></span><button onClick={()=>setAuthOpen(true)}>Use your own data <ArrowRight size={14}/></button></div>}
{logged&&data.wallets.some(w=>w.synthetic)&&<div className="preview-banner"><span><Eye size={15}/><strong>This database contains synthetic demo history.</strong><span> Demo results are not actual trading performance.</span></span></div>}
{(page==='overview'||page==='wallets')&&<><div className="metric-grid"><Metric label="Wallets analyzed" value={String(data.wallets.length).padStart(2,'0')} foot="Full imported trading histories" icon={<Wallet size={16}/>} trend="Dataset"/><Metric label="Realized wallet P&L" value={(aggregate>=0?'+':'')+number(aggregate)} unit="SOL" foot="Combined wallet results · not your returns" icon={<TrendingUp size={16}/>} positive/><Metric label="Trades indexed" value={number(data.wallets.reduce((s,w)=>s+w.trades,0),0)} foot="Deduplicated transaction events" icon={<Activity size={16}/>} trend="Historical"/><Metric label="Following" value={String(data.watches.length).padStart(2,'0')} foot={me?.enabled?'Paper strategy is enabled':'Your watchlist, your decisions'} icon={<Crosshair size={16}/>} trend={me?.enabled?'Enabled':'Standby'}/></div></>}
{page==='overview'&&<>{logged&&!data.wallets.length&&<section className="panel padded onboarding">
<span className="overline">NOTHING TO ANALYZE YET</span>
<h3>Your workspace is empty.</h3>
<p>The preview you saw before signing in was labelled sample data. Signing in switches to <strong>your</strong> dataset, and this one has no trade history in it yet — so there are no wallets to rank, no dossiers to open and nothing to copy.</p>
<div className="button-row">{me?.admin
 ? <button className="button primary" disabled={busy} onClick={()=>run(async()=>{const r=await api('/admin/import','POST',demoTrades);await refresh();setToast(`${r.inserted} synthetic demo events imported`)})}>Load synthetic demo history <ArrowRight size={15}/></button>
 : <button className="button" onClick={()=>navigate('wallets')}>See what a dossier looks like <ArrowRight size={15}/></button>}
<button className="text-button" onClick={()=>navigate('admin')}>{me?.admin?'Or import your own JSON':'Ask an administrator to import history'}</button></div>
<p className="fine-print">Demo events have IDs starting with <code>demo-</code> and stay labelled as synthetic everywhere they appear. They are not real wallet performance and must not be mixed into a production dataset.</p>
</section>}<div className="overview-grid"><section className="panel performance-panel"><div className="panel-top"><div><span className="overline">WALLET PERFORMANCE</span><h3>{leader?(logged?short(leader.wallet):'Quiet conviction'):'Waiting for a first wallet'} <span className="subtle-label">{logged?'Imported':'SAMPLE'}</span></h3></div><div className="segmented">{['7D','30D','ALL'].map(p=><button key={p} className={period===p?'selected':''} onClick={()=>setPeriod(p)}>{p}</button>)}</div></div><div className="chart-summary"><strong className="positive">{leader?(leader.realized_pnl_sol>=0?'+':'')+number(leader.realized_pnl_sol):'0.00'} <span>SOL</span></strong><span className="chart-caption"><span className="status-dot"/> Cumulative realized P&L · FIFO</span></div><div className="chart-area"><div className="chart-y">{[1,.75,.5,.25,0].map(n=><span key={n}>{number(Math.max(1,...curve.map(p=>p[1]))*n,0)}</span>)}</div><LineChart values={curve.map(p=>p[1])}/></div><div className="chart-x">{curve.filter((_,i)=>i===0||i===Math.floor(curve.length/3)||i===Math.floor(curve.length*2/3)||i===curve.length-1).map(p=><span key={p[0]}>{date(p[0])}</span>)}</div><div className="chart-footer"><span><span className="legend-line"/> Realized profit</span><span>After supplied fees <CircleHelp size={12}/></span></div></section><section className="panel conviction-panel"><div className="panel-top"><span className="overline">BEHIND THE SCORE</span><Crosshair size={17}/></div><h3>A good trade.<br/>Or a good trader?</h3><p>One big win isn’t the whole story. Trace the decisions that came before it.</p><div className="score-visual"><svg viewBox="0 0 200 110"><path d="M 22 94 A 78 78 0 0 1 178 94" fill="none" stroke="var(--line-2)" strokeWidth="9" strokeLinecap="round"/><path d="M 22 94 A 78 78 0 0 1 178 94" fill="none" stroke="var(--accent)" strokeWidth="9" strokeLinecap="round" pathLength="100" strokeDasharray={`${leader?.score||0} 100`}/></svg><div><strong>{leader?.score||'—'}</strong><span>HEURISTIC SCORE</span></div></div><div className="score-factors"><span>Track record <strong>{leader?.matched_sells||0} matched sells</strong></span><span>Win consistency <strong>{leader?.win_rate?number(leader.win_rate,1)+'%':'—'}</strong></span><span>Cost basis <strong>{leader&&leader.unmatched_quantity===0?'Matched history':'Incomplete'}</strong></span></div><button onClick={()=>leader?detail(leader):navigate('research')}>Explore the analysis <ArrowUpRight size={15}/></button></section></div><section className="panel wallets-panel"><SectionHeading title="Wallets worth a closer look" sub="Ranked by the evidence in their trading history."><button className="text-button" onClick={()=>navigate('wallets')}>Explore wallets <ArrowRight size={14}/></button></SectionHeading><WalletTable compact/></section><div className="bottom-grid"><section className="panel"><SectionHeading title="Recent wallet activity" sub="The latest events in your dataset."><button className="text-button" onClick={()=>navigate('signals')}>View all <ArrowRight size={14}/></button></SectionHeading><FeedList/></section><section className="panel strategy-card"><span className="strategy-icon"><Layers3 size={23}/></span><span className="overline">INTENTION BEFORE EXECUTION</span><h3>Follow the wallet.<br/>Keep your own limits.</h3><p>Set an allocation, follow a strategy, and inspect every paper entry and exit before going further.</p><div className="strategy-details"><span><Check size={14}/> Position limits</span><span><Check size={14}/> Leader exit detection</span><span><Check size={14}/> Daily loss guard</span></div><button className="button" onClick={()=>navigate('execution')}>Set up paper trading <ArrowRight size={15}/></button></section></div></>}
{page==='wallets'&&<section className="panel"><SectionHeading title="Wallet intelligence" sub="Scores describe the supplied history. They do not predict returns."><div className="table-tools"><label className="search"><Search size={15}/><input aria-label="Search wallets" placeholder="Search wallet address…" value={query} onChange={e=>setQuery(e.target.value)}/></label><select aria-label="Sort wallets" value={sort} onChange={e=>setSort(e.target.value)}><option value="score">Highest score</option><option value="pnl">Highest P&L</option></select></div></SectionHeading><WalletTable/></section>}
{page==='signals'&&logged&&<>
<section className="panel"><SectionHeading title="Your positions" sub="Paper positions, marked against the last observed tick."/>
<div className="table-scroll"><table><thead><tr><th>Token</th><th>Leader</th><th>Cost</th><th>Result</th><th>Status</th></tr></thead><tbody>
{(socialFeed?.positions||[]).map((p,i)=><tr key={p.token+p.created+i}><td><strong>{p.symbol}</strong></td><td><code>{short(p.wallet)}</code></td><td>{number(p.cost,4)} SOL</td>
<td className={((p.status==='open'?p.unrealized_pnl_sol:p.pnl)||0)>=0?'positive':'negative'}>
{p.status==='open'
 ? (p.unrealized_pnl_sol==null?<span className="subtle-label">no mark</span>:number(p.unrealized_pnl_sol,4)+' SOL unrealized')
 : (p.pnl==null?'—':number(p.pnl,4)+' SOL')}</td>
<td><span className={'tag '+(p.status==='open'?'amber':'green')}>{p.status.toUpperCase()}</span></td></tr>)}
</tbody></table></div>
{!socialFeed?.positions.length&&<Empty title="No positions yet" text="Follow a wallet and enable paper execution to see entries here."/>}</section>
<section className="panel"><SectionHeading title="What the wallets you follow just did" sub="From this service's own record of their trades."/>
<div className="feed-list">{(socialFeed?.events||[]).map(t=><div className="feed-row" key={t.id}>
<span className={'trade-icon '+t.side}>{t.side==='buy'?<ArrowDownLeft size={16}/>:<ArrowUpRight size={16}/>}</span>
<div className="feed-main"><strong>{short(t.wallet)} <span>{t.side}</span> {t.symbol}</strong><small>{date(t.timestamp)} · {number(t.quantity*t.price_sol,4)} SOL</small></div>
<button className="button small" onClick={()=>openToken(t.token)}>Open</button></div>)}</div>
{!socialFeed?.events.length&&<Empty title="Nothing from your watchlist yet" text="Follow wallets to see their trades as they are observed."/>}
<p className="fine-print">{socialFeed?.note}</p></section>
</>}
{page==='signals'&&!logged&&<section className="panel"><SectionHeading title="Signal feed" sub="Observed events, not recommendations. No live stream is connected."><div className="table-tools"><label className="search"><Search size={15}/><input aria-label="Search activity" placeholder="Search token or wallet…" value={query} onChange={e=>setQuery(e.target.value)}/></label><select aria-label="Filter activity" value={feedFilter} onChange={e=>setFeedFilter(e.target.value)}>{['All activity','Buys','Sells'].map(s=><option key={s}>{s}</option>)}</select></div></SectionHeading><FeedList full/></section>}
{page==='execution'&&me?.copy_trading===false&&<Empty title="Copy trading is switched off" text="The operator has disabled it on this platform. Swapping and token analysis are unaffected."/>}
{page==='execution'&&me?.copy_trading!==false&&<><div className="notice"><Shield size={18}/><span><strong>Paper execution only.</strong> Live trading is unavailable until a supported signing and execution integration is configured. Linking a wallet address does not authorize trades.</span></div><div className="execution-grid"><section className="panel padded"><span className="overline">STRATEGY CONTROLS</span><h3>Measured conviction</h3><p>Follow a wallet from the Smart wallets page, then replay its imported history.</p><label className="field">Order allocation <span className="input-unit"><input type="number" aria-label="Order allocation" min="0.01" max="10" step="0.01" value={budget} onChange={e=>setBudget(Number(e.target.value))}/><span>SOL</span></span></label><div className="rules"><span>Maximum open exposure <strong>{number(budget*5)} SOL</strong></span><span>Daily realized loss guard <strong>{number(budget*2)} SOL</strong></span><span>Minimum prior matched sells <strong>20</strong></span><span>Minimum heuristic score <strong>65 / 100</strong></span><span>Assumed cost per side <strong>2% + 0.00001 SOL</strong></span><span>Exit triggers <strong>Leader sell / 20% drop / thin liquidity</strong></span></div><div className="button-row"><button className="button primary" disabled={busy} onClick={()=>needAuth()&&run(async()=>{await api('/settings','POST',{enabled:!me?.enabled,budget,mode:'paper'});await refresh();setToast(me?.enabled?'Paper execution paused':'Paper execution enabled')})}>{me?.enabled?<Pause size={15}/>:<Play size={15}/>} {me?.enabled?'Pause execution':'Enable paper execution'}</button><button className="button" disabled={busy||!me?.enabled} onClick={()=>run(async()=>{await api('/settings','POST',{enabled:true,budget,mode:'paper'});const r=await api('/paper/replay','POST');await refresh();setToast(`${r.opened} paper entries · ${r.closed} paper exits`)})}>Replay history</button></div><p className="fine-print">Replay is incremental and never sends real transactions. Fixed execution costs do not model real fill prices or latency. Stop checks occur at observed events, not continuously.</p></section><section className="panel padded"><span className="overline">PAPER ACCOUNT</span><h3 className={pnl>=0?'positive':'negative'}>{pnl>=0?'+':''}{number(pnl)} SOL</h3><p>Realized paper P&L</p><div className="rules"><span>Open positions<strong>{data.positions.filter(p=>p.status==='open').length}</strong></span><span>Closed positions<strong>{data.positions.filter(p=>p.status==='closed').length}</strong></span><span>Following<strong>{data.watches.length} wallets</strong></span><span>Execution status<strong>{me?.enabled?'Enabled':'Paused'}</strong></span><span>Subscription<strong>{me?.subscribed?'Active':'Required'}</strong></span></div><div className="inset-note"><Eye size={20}/><p>Profitable source wallets can produce losing copy trades. Your entry price, execution delay, and available liquidity will differ.</p></div></section></div><section className="panel"><SectionHeading title="Paper positions" sub="Every entry and exit retains its reason."/>{!data.positions.length?<Empty title="A clean slate" text="Follow a wallet, enable paper execution, and replay imported history to see qualifying positions."/>:<div className="table-scroll"><table><thead><tr><th>Token</th><th>Leader</th><th>Allocated</th><th>P&L</th><th>Reason</th><th>Status</th></tr></thead><tbody>{data.positions.map(p=><tr key={p.id}><td><strong>{p.symbol}</strong></td><td>{short(p.wallet)}</td><td>{number(p.cost)} SOL</td><td className={(p.pnl||0)>=0?'positive':'negative'}>{p.pnl===null?'—':number(p.pnl)+' SOL'}</td><td>{p.reason}</td><td>{p.status==='open'?<button className="button small" disabled={busy} onClick={()=>run(async()=>{await api(`/positions/${p.id}/close`,'POST');await refresh()})}>Close paper position</button>:<span className="tag muted">CLOSED</span>}</td></tr>)}</tbody></table></div>}</section></>}
{page==='discover'&&(logged?<>
<section className="panel"><div className="market-bar"><div className="segmented">{[['mcap','Market cap'],['volume','Volume'],['change','24h'],['new','Newest']].map(([k,l])=><button key={k} className={sortBy===k?'selected':''} onClick={()=>setSortBy(k)}>{l}</button>)}</div>
<span className="market-note">A listing is not an endorsement.</span>
<button className="text-button" disabled={busy} onClick={loadDiscover}>Refresh <ArrowRight size={14}/></button></div>
<div className="token-grid">{[...(discovered?.profiled||[])].sort((a,b)=>{
 // Tokens with no market always sort last: they have nothing to rank on.
 if(!a.market||!b.market)return (a.market?0:1)-(b.market?0:1);
 const f=(x:typeof a)=>sortBy==='volume'?(x.market!.volume_h24||0)
  :sortBy==='change'?(x.market!.price_change_h24||0)
  :sortBy==='new'?-(x.market!.pair_created_at||0)
  :(x.market!.market_cap||x.market!.liquidity_usd||0);
 return f(b)-f(a)}).map(t=><button className="token-card" key={t.mint} onClick={()=>openToken(t.mint,{icon:t.icon,description:t.description})}>
{t.icon?<img src={t.icon} alt="" loading="lazy"/>:<span className="token-fallback"><Fingerprint size={20}/></span>}
<div className="token-card-main"><strong>{t.market?.symbol||short(t.mint)}</strong>
<small>{t.market?.name||(t.description||'No description').slice(0,48)}</small>
{t.market
 ?<span className="token-card-meta"><b>{compact(t.market.market_cap??t.market.liquidity_usd)}</b>
   <em className={(t.market.price_change_h24||0)>=0?'positive':'negative'}>{pct(t.market.price_change_h24)}</em>
   <i>vol {compact(t.market.volume_h24)}</i></span>
 :<span className="token-card-meta"><i>not trading yet</i></span>}</div>
{age(t.market?.pair_created_at)&&<span className="token-card-age">{age(t.market?.pair_created_at)}</span>}</button>)}</div>
{!discovered?.profiled.length&&<Empty title="Nothing profiled right now" text="The provider returns a rolling list; try refreshing."/>}<p className="fine-print grid-note">{discovered?.note}</p></section>
<section className="panel"><SectionHeading title="Launches we saw ourselves" sub="Mints observed directly from the creation stream, newest first."/>
<div className="table-scroll"><table><thead><tr><th>Token</th><th>Mint</th><th>X account</th><th>Seen</th><th></th></tr></thead><tbody>
{(discovered?.observed||[]).map(t=><tr key={t.mint}><td><strong>{t.symbol||'—'}</strong> <span className="subtle-label">{t.name||''}</span>{(t.copies||1)>1&&<span className="tag repeat-tag" title={`This deployer minted ${t.copies} tokens with this same name. Only the newest is listed.`}>×{t.copies} same deployer</span>}</td><td><code>{short(t.mint)}</code></td><td>{t.twitter?<a href={t.twitter} target="_blank" rel="noreferrer noopener">{t.twitter.replace(/^https?:\/\/(www\.)?/,'').slice(0,28)}</a>:<span className="subtle-label">none</span>}</td><td>{date(t.first_seen)}</td><td><button className="button small" onClick={()=>openToken(t.mint,{name:t.symbol||t.name||undefined})}>Open</button></td></tr>)}
</tbody></table></div>
{!discovered?.observed.length&&<Empty title="No launches observed yet" text="Enable the discovery collector to record new mints as they are created."/>}</section>
</>:<Empty title="Sign in to browse markets" text="Token pages combine live price, safety checks and the account promoting the coin."/>)}

{page==='token'&&<>
<section className="panel padded token-head">
<div className="token-identity">{(market?.image_url||hint?.icon)?<img src={market?.image_url||hint?.icon} alt=""/>:<span className="token-fallback large"><Fingerprint size={26}/></span>}
<div><h2>{market?.symbol||hint?.name||'Not trading yet'} <span className="subtle-label">{market?.name||hint?.description?.slice(0,60)||''}</span></h2>
<button className="address-copy" onClick={()=>run(async()=>{await navigator.clipboard.writeText(mint);setToast('Mint copied')})}>{mint}<Copy size={13}/></button></div></div>
<div className="token-stats">
<div><span>Price</span><strong>{usd(market?.price_usd,8)}</strong></div>
<div><span>24h</span><strong className={(market?.price_change_h24||0)>=0?'positive':'negative'}>{market?.price_change_h24==null?'—':number(market.price_change_h24,2)+'%'}</strong></div>
<div><span>Liquidity</span><strong>{usd(market?.liquidity_usd,0)}</strong></div>
<div><span>24h volume</span><strong>{usd(market?.volume_h24,0)}</strong></div>
<div><span>Market cap</span><strong>{usd(market?.market_cap,0)}</strong></div>
<div><span>Buys / sells</span><strong>{market?.buys_h24??'—'} / {market?.sells_h24??'—'}</strong></div></div>
<p className="fine-print">{market
 ?`Deepest of ${market.pools} pool${market.pools===1?'':'s'} on ${market.dex||'an unknown venue'}, as of ${marketAge}s ago. Not a live tick.`
 :'No market exists for this mint yet. It appears in the promoted listing but no pool has been indexed, so there is no price, no depth and nothing to chart. That is the state of the token, not a failure to load.'}</p>
{(market?.warnings||[]).map(w=><div className="flag" key={w}><CircleHelp size={15}/><span>{w}</span></div>)}
</section>

<section className="panel padded chart-panel">
<div className="chart-top"><div><span className="overline">PRICE</span>
<div className="chart-legend"><span><i className="up"/>Close at or above open</span><span><i className="down"/>Close below open</span></div></div>
<div className="chart-controls">
<div className="segmented">{(['price','mcap'] as const).map(k=><button key={k} className={scale===k?'selected':''} disabled={k==='mcap'&&!capMultiplier()} onClick={()=>setScale(k)}>{k==='price'?'Price':'MCap'}</button>)}</div>
<div className="segmented">{(candles?.timeframes||['5m','1h','4h','1d']).map(k=><button key={k} className={tf===k?'selected':''} onClick={()=>loadCandles(k)}>{k}</button>)}</div></div></div>
{candles?.candles.length?<Candles data={candles.candles} multiplier={scale==='mcap'?(capMultiplier()||1):1}/>:<Empty title={market?'No candle history yet':'Nothing to chart'} text={market?'The pool exists but the provider has no candles for it yet.':'No pool has been indexed for this mint, so there is no price history to draw.'}/>}
{candles&&<p className="fine-print">As of {candles.age_seconds}s ago. {candles.note}{scale==='mcap'?' Market cap is the price scaled by today\u2019s circulating supply, so earlier points assume supply has not changed.':''}</p>}
</section>

<div className="token-layout">
<section className="panel padded buy-panel">
<div className="segmented side-toggle"><button className={side==='buy'?'selected':''} onClick={()=>{setSide('buy');setQuote(null)}}>Buy</button><button className={side==='sell'?'selected':''} onClick={()=>{setSide('sell');setQuote(null);if(!portfolio)run(async()=>setPortfolio(await api('/portfolio')))}}>Sell</button></div>
{side==='buy'
 ? <><label className="field">Amount in SOL<input type="number" min="0.001" step="0.001" value={buySol} onChange={e=>setBuySol(Number(e.target.value))}/></label>
   <div className="segmented">{[0.1,0.5,1,5].map(v=><button key={v} className={buySol===v?'selected':''} onClick={()=>{setBuySol(v);setQuote(null)}}>{v}</button>)}</div></>
 : <><div className="rules"><span>You hold<strong>{heldAmount()?number(heldAmount()!.amount,4):(portfolio?'none of this token':(busy?'loading…':'unavailable'))}</strong></span></div>
   <div className="segmented">{[25,50,75,100].map(p=><button key={p} className={sellPct===p?'selected':''} onClick={()=>{setSellPct(p);setQuote(null)}}>{p}%</button>)}</div></>}
<label className="field">Max slippage<select value={slippage} onChange={e=>setSlippage(Number(e.target.value))}><option value={50}>0.5%</option><option value={100}>1%</option><option value={300}>3%</option><option value={500}>5%</option></select></label>
<div className="button-row"><button className="button" disabled={busy||!mint||!market} onClick={priceSwap}>Get price</button>
<button className="button primary" disabled={busy||!mint||!market} onClick={executeSwap}>{side==='buy'?'Buy':'Sell'} <ArrowRight size={15}/></button></div>
{quote&&<div className="rules"><span>You receive<strong>{side==='buy'?number(quote.out_amount)+' units':number(quote.out_amount/1e9,6)+' SOL'}</strong></span>
<span>At worst<strong>{number(quote.minimum_out)} units</strong></span>
<span>Price impact<strong>{quote.price_impact_pct==null?'unknown':number(quote.price_impact_pct*100,2)+'%'}</strong></span>
<span>Platform fee<strong>{quote.platform_fee_bps/100}%</strong></span>
<span>Route<strong>{quote.route.join(' → ')||'—'}</strong></span></div>}
{(quote?.warnings||[]).map(w=><div className="flag" key={w}><CircleHelp size={15}/><span>{w}</span></div>)}
{!market&&<p className="fine-print">Trading is unavailable: this mint has no indexed market to route through.</p>}
<p className="fine-print">Your wallet signs and broadcasts. This service never holds the key that signs a trade, and a price is only final once the network fills it.</p>
</section>

<section className="panel padded">
<h3>The account behind it</h3>
{tokenPage?.social?<>
<div className="rules"><span>Handle<strong>@{tokenPage.social.handle}</strong></span>
<span>Tokens fronted<strong>{tokenPage.social.tokens_promoted}</strong></span>
<span>Followers<strong>{tokenPage.social.followers??'unknown'}</strong></span>
<span>Identity resolved<strong>{tokenPage.social.identity_resolved?'yes':'no'}</strong></span></div>
{tokenPage.social.flags.map(f=><div className="flag" key={f}><CircleHelp size={15}/><span>{f}</span></div>)}
{!!tokenPage.social.previous_mints.length&&<><h3>Previously promoted</h3><div className="drawer-history">{tokenPage.social.previous_mints.map(m=><button className="feed-row" key={m} onClick={()=>openToken(m)}><code>{short(m)}</code></button>)}</div></>}
</>:<Empty title="No X account attached" text="This token's metadata names no social account, so there is nothing to check its history against."/>}
{!!market?.socials.length&&<div className="button-row">{market.socials.map(([kind,url])=><a className="button small" key={url} href={url} target="_blank" rel="noreferrer noopener">{kind}</a>)}{market.websites.map(u=><a className="button small" key={u} href={u} target="_blank" rel="noreferrer noopener">website</a>)}</div>}
<h3>Stats</h3>
{market?<><div className="rules">
<span>Market cap<strong>{compact(market.market_cap)}</strong></span>
<span>Fully diluted<strong>{compact(market.fdv)}</strong></span>
<span>Liquidity<strong>{compact(market.liquidity_usd)}</strong></span>
<span>24h volume<strong>{compact(market.volume_h24)}</strong></span>
<span>Pool age<strong>{market.pair_created_at?date(market.pair_created_at):'—'}</strong></span>
<span>Venue<strong>{market.dex||'—'}</strong></span></div>
{(market.buys_h24!=null||market.sells_h24!=null)&&(()=>{const b=market.buys_h24||0,x=market.sells_h24||0,tot=b+x;
 return <div className="flow"><div className="flow-heads"><span className="positive">{b.toLocaleString()} buys</span><span className="negative">{x.toLocaleString()} sells</span></div>
 <div className="flow-bar"><i className="up" style={{width:(tot?b/tot*100:50)+'%'}}/><i className="down" style={{width:(tot?x/tot*100:50)+'%'}}/></div></div>})()}
</>:<Empty title="No market" text="Nothing is trading, so there are no statistics to show."/>}
<h3>Holders</h3>
{holders?.holders?.length?<>
<div className="table-scroll"><table><thead><tr><th>Token account</th><th>Amount</th><th>Share</th></tr></thead><tbody>
{holders.holders.slice(0,10).map(hh=><tr key={hh.address}><td><code>{short(hh.address)}</code></td><td>{number(hh.amount,2)}</td><td>{hh.share_pct==null?'—':number(hh.share_pct,2)+'%'}</td></tr>)}
</tbody></table></div>
<p className="fine-print">{holders.note}</p>
{!!holders.top_traders?.length&&<><h3>Traders we have seen here</h3>
<div className="table-scroll"><table><thead><tr><th>Wallet</th><th>Realized</th><th>Round trips</th><th>Score</th></tr></thead><tbody>
{holders.top_traders.slice(0,8).map(t=><tr key={t.wallet}><td><code>{short(t.wallet)}</code></td><td className={t.realized_pnl_sol>=0?'positive':'negative'}>{number(t.realized_pnl_sol)} SOL</td><td>{t.round_trips}{t.round_trip_win_rate==null?'':' · '+number(t.round_trip_win_rate,0)+'%'}</td><td><span className="score high">{t.score}</span></td></tr>)}
</tbody></table></div></>}
</>:<Empty title="Holders not loaded" text="The chain did not return holder accounts for this mint."/>}
<h3>Safety</h3>
{tokenPage?.risk?<div className="flag"><CircleHelp size={15}/><span>{tokenPage.risk.blocked||`Cleared the pre-entry gate at score ${tokenPage.risk.score??'unknown'}`}</span></div>
:<Empty title="No safety report collected" text="Unknown, not clean. An administrator can collect one for this mint."/>}
</section></div></>}

{page==='funds'&&(logged?<>
<div className="metric-grid"><Metric label="Wallet balance" value={funds?.balance_sol==null?'—':number(funds.balance_sol,4)+' SOL'} foot={funds?.balance_error?'Provider unavailable; this is not a zero balance':'On-chain, confirmed'} icon={<Wallet size={16}/>}/>
<Metric label="Realized P&L" value={number(funds?.realized_pnl_sol||0)+' SOL'} foot="Paper results; no order has been signed" icon={<Activity size={16}/>}/>
<Metric label="Commission owed" value={number((funds?.commission_owed_lamports||0)/1e9,4)+' SOL'} foot="Settled when you withdraw" icon={<CreditCard size={16}/>}/>
<Metric label="Withdrawable" value={funds?.withdrawable_lamports==null?'—':number(funds.withdrawable_lamports/1e9,4)+' SOL'} foot={'Reserves of '+number((funds?.reserved_lamports||0)/1e9,5)+' SOL are withheld'} icon={<Layers3 size={16}/>}/></div>
<section className="panel padded"><h3>Deposit</h3><p>{funds?.deposit_note}</p>
{funds?.deposit_address?<button className="address-copy" onClick={()=>run(async()=>{await navigator.clipboard.writeText(funds.deposit_address!);setToast('Deposit address copied')})}>{funds.deposit_address}<Copy size={13}/></button>:<Empty title="No managed wallet" text="Custody is not configured on this server, so no trading wallet was created for this account."/>}
<p className="fine-print">This wallet is held by this service. We can sign for it until you close your account, at which point the private key is handed to you once and we can no longer sign for it. That makes this service a custodian of your funds.</p></section>
<section className="panel padded"><h3>Withdraw</h3>
{funds?.withdrawals_enabled?<form onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);run(async()=>{const r=await api('/funds/withdraw','POST',{destination:f.get('destination'),lamports:Math.round(Number(f.get('amount'))*1e9)});await loadFunds();setToast('Withdrawal submitted: '+r.signature)})}}>
<div className="form-grid"><label className="field">Destination Solana address<input name="destination" required minLength={32} maxLength={44}/></label><label className="field">Amount in SOL<input name="amount" type="number" min="0.000001" step="0.000001" required/></label></div>
<button className="button primary" disabled={busy}>Withdraw <ArrowRight size={15}/></button></form>
:<p>Withdrawals are not enabled on this server.</p>}
<p className="fine-print">Outstanding commission is settled before a withdrawal, and the network reserve is withheld so the account stays rent exempt. A withdrawal that times out is recorded as unconfirmed and may still land — check the signature before trying again, because retrying sends a second payment.</p></section>
<section className="panel padded"><h3>How we get paid</h3><p>{funds?.conflict_note}</p>
{feeInfo&&<div className="rules"><span>Model<strong>{feeInfo.schedule.model}</strong></span><span>Share of new profit<strong>{feeInfo.schedule.performance_bps/100}%</strong></span><span>Per-trade<strong>{feeInfo.schedule.trade_bps/100}%</strong></span><span>High-water mark<strong>{number(feeInfo.high_water_lamports/1e9,4)} SOL</strong></span><span>Charged to date<strong>{number(feeInfo.total_charged_lamports/1e9,4)} SOL</strong></span><span>Network cost assumed per trade<strong>{number(feeInfo.cost_model.fixed_lamports/1e9,5)} SOL + {feeInfo.cost_model.bps/100}%</strong></span></div>}
{!!feeInfo?.ledger.length&&<div className="table-scroll"><table><thead><tr><th>When</th><th>Kind</th><th>Amount</th><th>On</th></tr></thead><tbody>{feeInfo.ledger.map(f=><tr key={f.reference}><td>{date(f.created)}</td><td>{f.kind}</td><td>{number(f.lamports/1e9,5)} SOL</td><td>{number(f.basis_lamports/1e9,4)} SOL</td></tr>)}</tbody></table></div>}</section>
<section className="panel padded"><h3>Export your key</h3>
<p>Take a copy of your wallet's private key at any time, the way Axiom and Trojan let you. This does <strong>not</strong> end custody: we still hold the key and can still sign. For sole control, move the funds to a wallet we never generated, or close the account below.</p>
<button className="button" disabled={busy} onClick={()=>run(async()=>{setReleased(await api('/funds/export-key','POST',{confirm:'EXPORT'}))})}>Reveal private key</button></section>
<section className="panel padded"><h3>Close account</h3><p>Closing releases your private key to you, once. Afterwards this service cannot sign for the wallet and your sessions end. Close any open positions first.</p>
<button className="button" disabled={busy} onClick={()=>run(async()=>{setReleased(await api('/account/close','POST',{confirm:'CLOSE'}))})}>Close account and release my key</button>
{released&&<><p className="notice">{released.warning}</p><pre className="code-block">{released.secret_key}</pre><button className="text-button" onClick={()=>setReleased(null)}>Hide</button></>}</section>
</>:<Empty title="Sign in to see your funds" text="Every account gets a Solana wallet held here until the account is closed."/>)}
{page==='portfolio'&&(logged?<>
<div className="metric-grid">
<Metric label="SOL" value={portfolio?.sol_lamports==null?'—':number(portfolio.sol_lamports/1e9,4)} foot={portfolio?.sol_error?'Balance unavailable, not zero':'On chain, confirmed'} icon={<Wallet size={16}/>}/>
<Metric label="Token value" value={usd(portfolio?.valued_usd,2)} foot={portfolio?.unpriced_tokens?`${portfolio.unpriced_tokens} holding(s) could not be priced and are excluded`:'All holdings priced'} icon={<Activity size={16}/>}/>
<Metric label="Holdings" value={String(portfolio?.tokens.length??0)} foot="Non-zero token accounts" icon={<Layers3 size={16}/>}/>
<Metric label="Wallet" value={portfolio?short(portfolio.address):'—'} foot="Linked Solana address" icon={<CreditCard size={16}/>}/></div>
{portfolio?.holdings_error&&<div className="notice">{portfolio.holdings_error}</div>}
<section className="panel"><SectionHeading title="Holdings" sub="Straight from the chain. A token we cannot price shows no value rather than zero."><button className="text-button" disabled={busy} onClick={()=>run(async()=>setPortfolio(await api('/portfolio')))}>Refresh <ArrowRight size={14}/></button></SectionHeading>
<div className="table-scroll"><table><thead><tr><th>Token</th><th>Amount</th><th>Price</th><th>Value</th><th></th></tr></thead><tbody>
{(portfolio?.tokens||[]).map(t=><tr key={t.mint}><td><code>{short(t.mint)}</code></td><td>{number(t.amount,4)}</td><td>{t.price_usd==null?<span className="subtle-label">unpriced</span>:usd(t.price_usd,8)}</td><td>{t.value_usd==null?'—':usd(t.value_usd,2)}</td><td><button className="button small" onClick={()=>openToken(t.mint)}>Open</button></td></tr>)}
</tbody></table></div>
{!portfolio?.tokens.length&&<Empty title="No token holdings" text="This wallet holds no non-zero token accounts."/>}
<p className="fine-print">{portfolio?.note}</p></section>
</>:<Empty title="Sign in to see your portfolio" text="Holdings are read from the chain for the wallet linked to your account."/>)}

{page==='settings'&&<><div className="notice"><Link2 size={18}/><span><strong>Connection readiness, without guesswork.</strong> These are integration targets. No platform account is connected and no account credentials are requested.</span></div><div className="connection-grid">{[{name:'Axiom',mark:'▲',label:'Authorization unverified',url:'https://docs.axiom.trade/',text:'A trading terminal with wallet tracking. A supported third-party account authorization flow has not been established from the official documentation reviewed.'},{name:'Fomo',mark:'f.',label:'Authorization unverified',url:'https://fomo.family/',text:'Social trading and wallet activity. A supported public API for delegated account trading has not been established from the official material reviewed.'},{name:'Pump.fun',mark:'◒',label:'Onchain protocol documented',url:'https://github.com/pump-fun/pump-public-docs',text:'Public program documentation is available. Trading requires Solana connectivity, user-authorized signing, and user-paid transaction fees.'}].map(c=><section className="panel connection-card" key={c.name}><span className="platform-mark">{c.mark}</span><h3>{c.name}</h3><span className="tag amber">{c.label}</span><p>{c.text}</p><a className="button" href={c.url} target="_blank" rel="noreferrer">Official documentation <ExternalLink size={14}/></a><small>Live connection unavailable</small></section>)}</div><section className="panel padded"><h3>Your wallet watchlist</h3><p>Track public addresses without claiming ownership or sharing private keys.</p><form className="inline-form" onSubmit={e=>{e.preventDefault();const form=new FormData(e.currentTarget);follow(String(form.get('wallet')))}}><input name="wallet" placeholder="Public wallet address" required maxLength={128} aria-label="Public wallet address"/><button className="button primary" disabled={busy}><Plus size={15}/> Add to watchlist</button></form><div className="watch-chips">{data.watches.map(w=><button key={w} onClick={()=>follow(w)}>{short(w)} <X size={12}/></button>)}</div></section></>}
{page==='settings'&&<><div className="execution-grid"><section className="panel padded"><span className="overline">YOUR API ACCESS</span><h3>Make the signal yours.</h3><p>Query the same wallet analysis used in this workspace. Keys expire after 30 days. Creating a key revokes the previous one.</p><div className="code-block">{apiKey||'Your API key will appear once after creation.'}</div><div className="button-row"><button className="button primary" disabled={busy} onClick={()=>needAuth()&&run(async()=>{const r=await api('/keys','POST');setApiKey(r.key);setToast('API key created. Store it securely.')})}><Plus size={15}/>Create / rotate key</button>{apiKey&&<button className="button" onClick={()=>run(async()=>{await navigator.clipboard.writeText(apiKey);setToast('Copied to clipboard')})}><Copy size={14}/>Copy</button>}<button className="button" disabled={busy||!me} onClick={()=>run(async()=>{await api('/keys','DELETE');setApiKey('');setToast('API access revoked')})}>Revoke</button></div><p className="fine-print">An active subscription is required for detailed analysis. API keys cannot access administration.</p></section><section className="panel padded"><span className="overline">QUICK START</span><h3>One request. The full picture.</h3><pre className="code-block">{`curl http://localhost:8080/api/wallets/WALLET \\\n  -H "Authorization: Bearer $UNSKILLED_API_KEY"`}</pre><div className="rules"><span>Method<strong>GET</strong></span><span>Response<strong>JSON</strong></span><span>Cost basis<strong>FIFO, including supplied fees</strong></span><span>Data source<strong>Imported normalized history</strong></span></div></section></div><section className="panel padded"><h3>What comes back</h3><pre className="code-block">{JSON.stringify({wallet:'…',realized_pnl_sol:12.4,win_rate:62.5,matched_sells:24,profit_factor:1.8,max_drawdown_sol:3.1,unmatched_quantity:0,score:71,flags:['Historical observations; score is not a probability of profit'],curve:[[1750000000,1.2]],history:[]},null,2)}</pre></section></>}
{page==='billing'&&<><div className="notice"><CreditCard size={18}/><span>Plans are managed locally. Payment checkout is not connected. Administrators can record externally verified payments or complimentary access.</span></div><div className="connection-grid">{(plans.length?plans:[{id:'observer',name:'Observer',price_cents:1900,days:30,active:1},{id:'operator',name:'Operator',price_cents:4900,days:30,active:1},{id:'api',name:'API access',price_cents:9900,days:30,active:1}]).map((p,i)=><section className={'panel price-card '+(i===1?'featured':'')} key={p.id}><span className="overline">{p.name.toUpperCase()}</span><h3>€{number(p.price_cents/100,0)}<small> / {p.days} days</small></h3><p>Current local access package</p><ul><li><Check size={15}/>Wallet history & analysis</li><li><Check size={15}/>Paper strategy replay</li><li><Check size={15}/>Personal API key</li></ul><button className={'button '+(i===1?'primary':'')} onClick={()=>me?setToast('Contact the workspace administrator for access. Checkout is not configured.'):setAuthOpen(true)}>{me?.subscribed?'Manage access':'Request access'}<ArrowUpRight size={15}/></button></section>)}</div>{me&&<section className="panel padded"><h3>Your subscription</h3><p>{me.subscribed?'Active until '+new Date(me.subscription_expires*1000).toLocaleDateString():'No active subscription. You can still build a watchlist.'}</p></section>}</>}
{page==='admin'&&(me?.admin&&admin?<><div className="metric-grid admin-metrics"><Metric label="Registered accounts" value={String(admin.users.length)} foot="Includes administrators" icon={<Users size={16}/>}/><Metric label="Recorded revenue" value={'€'+number(admin.revenue_cents/100)} foot="Manually verified payments only" icon={<CreditCard size={16}/>}/><Metric label="Payment records" value={String(admin.payments.length)} foot="Latest 100 records, including free grants" icon={<Activity size={16}/>}/><Metric label="Active plans" value={String(admin.plans.filter(p=>p.active).length)} foot="Available subscription packages" icon={<Layers3 size={16}/>}/></div><section className="panel padded"><h3>Trade history ingestion</h3><p>Import normalized JSON trade events. Historical imports never trigger live or automatic orders.</p><div className="button-row"><label className="button"><Download size={15}/>Import JSON<input type="file" accept=".json,application/json" hidden onChange={e=>{const f=e.target.files?.[0];if(f)run(async()=>{const r=await api('/admin/import','POST',JSON.parse(await f.text()));await refresh();setToast(`${r.inserted} new events imported`)});e.target.value=''}}/></label><button className="button" disabled={busy} onClick={()=>run(async()=>{const r=await api('/admin/import','POST',demoTrades);await refresh();setToast(`${r.inserted} synthetic demo events imported`)})}>Load synthetic demo history</button></div><p className="fine-print">Demo events have IDs starting with demo-. Do not mix them into a production dataset.</p></section><section className="panel padded"><h3>Collect real Solana history</h3><p>Fetch one page of 20 finalized transactions using the server’s RPC endpoint. Only conservative, direct Pump/PumpSwap SOL swaps are converted. Other transactions remain archived.</p><form onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);run(async()=>{const result=await api('/admin/rpc/sync','POST',{wallet:f.get('wallet'),before:String(f.get('before')||'')||null});setRpcResult(result);await refresh();setToast(`${result.inserted} new trade observations imported`)})}}><div className="form-grid"><label className="field">Public Solana wallet<input name="wallet" required minLength={32} maxLength={44} placeholder="Wallet address"/></label><label className="field">Before signature (optional pagination)<input name="before" maxLength={100} placeholder="Paste next_before from the previous page"/></label></div><button className="button" disabled={busy}><Download size={15}/>{busy?'Collecting…':'Fetch transaction page'}</button></form>{rpcResult&&<pre className="code-block">{JSON.stringify(rpcResult,null,2)}</pre>}<p className="fine-print">No paid provider is required. Shared public RPC can reject or rate-limit requests. This is partial wallet history, not a global scanner. Unknown liquidity blocks paper entries for these observations.</p></section><section className="panel"><SectionHeading title="User access" sub="Bans immediately revoke sessions and stop paper execution."/><div className="table-scroll"><table><thead><tr><th>User</th><th>Role</th><th>Access</th><th>Subscription</th><th></th></tr></thead><tbody>{admin.users.map(u=><tr key={u.id}><td>{u.email}</td><td>{u.admin?'Administrator':'Member'}</td><td><span className={'tag '+(u.banned?'amber':'green')}>{u.banned?'BANNED':'ACTIVE'}</span></td><td>{u.expires>Date.now()/1000?date(u.expires):'Inactive'}</td><td>{!u.admin&&<button className="button small" disabled={busy} onClick={()=>run(async()=>{await api(`/admin/users/${u.id}/ban`,'POST',{banned:!u.banned});await refresh()})}>{u.banned?'Restore access':'Ban user'}</button>}</td></tr>)}</tbody></table></div></section><div className="execution-grid"><section className="panel padded"><h3>Record subscription access</h3><p>Mark paid only after verifying payment outside this application.</p><form onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);run(async()=>{await api('/admin/subscriptions','POST',{user_id:f.get('user'),plan_id:f.get('plan'),reference:f.get('reference'),paid:f.get('paid')==='on'});await refresh();setToast('Subscription recorded')})}}><label className="field">Account<select name="user">{admin.users.filter(u=>!u.banned).map(u=><option value={u.id} key={u.id}>{u.email}</option>)}</select></label><label className="field">Plan<select name="plan">{admin.plans.filter(p=>p.active).map(p=><option value={p.id} key={p.id}>{p.name}</option>)}</select></label><label className="field">Unique payment / grant reference<input name="reference" required maxLength={128}/></label><label className="checkbox"><input type="checkbox" name="paid"/>Payment independently verified</label><button className="button primary" disabled={busy}>Record access <Check size={15}/></button></form></section><section className="panel padded"><h3>Create or update a plan</h3><p>Reuse an existing plan ID to update its details.</p><form onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);run(async()=>{await api('/admin/plans','POST',{id:f.get('id'),name:f.get('name'),price_cents:Math.round(Number(f.get('price'))*100),days:Number(f.get('days')),active:f.get('active')==='on'});await refresh();setPlans(await api('/plans'));setToast('Plan saved')})}}><div className="form-grid"><label className="field">Plan ID<input name="id" required maxLength={40} placeholder="operator"/></label><label className="field">Display name<input name="name" required maxLength={80} placeholder="Operator"/></label><label className="field">Price in EUR<input name="price" type="number" min="0" max="100000" step=".01" required/></label><label className="field">Duration in days<input name="days" type="number" min="1" max="365" defaultValue="30" required/></label></div><label className="checkbox"><input name="active" type="checkbox" defaultChecked/>Available to users</label><button className="button primary" disabled={busy}>Save plan <Check size={15}/></button></form></section></div><section className="panel padded"><h3>Copy trading</h3>
<p>A separate product from swapping, with its own legal weight. Turning it off disables paper execution for every account, stands down anyone who had it enabled, and stops the live collector opening positions. Swapping is unaffected.</p>
<div className="button-row"><button className="button" disabled={busy} onClick={()=>run(async()=>{const r=await api('/admin/platform','POST',{copy_trading:me?.copy_trading===false});await refresh();setToast(r.note)})}>{me?.copy_trading===false?'Turn copy trading on':'Turn copy trading off'}</button>
<span className={'tag '+(me?.copy_trading===false?'amber':'green')}>{me?.copy_trading===false?'OFF':'ON'}</span></div></section>
<section className="panel"><SectionHeading title="Audit trail" sub="The latest 100 events. Credentials and private keys are never logged."/><div className="table-scroll"><table><thead><tr><th>Time</th><th>Action</th><th>Actor</th><th>Details</th></tr></thead><tbody>{admin.logs.map(l=><tr key={l.id}><td>{new Date(l.created*1000).toLocaleString()}</td><td><code>{l.action}</code></td><td>{short(l.user_id||'system')}</td><td>{l.detail}</td></tr>)}</tbody></table></div></section></>:<Empty title="Administrator access required" text="Sign in with the administrator account configured on the server."/>)}
{page==='research'&&<div className="research-grid">{[{n:'01',title:'Profit needs a cost basis.',text:'The Rust engine uses FIFO lots, apportions buy and sell fees, and excludes unmatched inventory from realized profit. Transfers, airdrops, missing history, and other wallets can make a profitable-looking address misleading.'},{n:'02',title:'A wallet is not a person.',text:'A trader can split positions across many wallets or exchanges. Wallet-level history does not reveal total wealth, identity, hedges, or privileged access. Funding links are evidence to investigate, not proof of common ownership.'},{n:'03',title:'Following changes the trade.',text:'A leader’s fill is not your fill. Latency, slippage, priority fees, failed transactions, and other followers can erase an apparent edge. Replay currently assumes 2% execution costs per side; it is not a realistic market simulator.'},{n:'04',title:'Scores are not predictions.',text:'The current score combines observed win rate, matched sell count, and positive realized profit, with a penalty for incomplete cost basis. It has no calibrated predictive accuracy. Future validation must use chronological out-of-sample data.'},{n:'05',title:'An exit signal needs an exit.',text:'Leader sells, price drops, and liquidity deterioration can be observed too late. A triggered stop cannot guarantee a fill. Paper replay evaluates these conditions only when a followed wallet event arrives.'},{n:'06',title:'Connectivity is infrastructure.',text:'No paid data API is required by this build. Live data still needs a platform feed or Solana RPC and an indexer. Official Solana guidance says shared public RPC endpoints are not intended for production applications.'}].map(r=><section className="panel research-card" key={r.n}><span>{r.n}</span><h3>{r.title}</h3><p>{r.text}</p></section>)}<section className="panel padded research-sources"><h3>Primary sources</h3><a href="https://solana.com/docs/core/fees" target="_blank" rel="noreferrer">Solana transaction fees <ExternalLink size={14}/></a><a href="https://solana.com/docs/references/clusters" target="_blank" rel="noreferrer">Solana RPC guidance <ExternalLink size={14}/></a><a href="https://github.com/pump-fun/pump-public-docs" target="_blank" rel="noreferrer">Pump.fun program documentation <ExternalLink size={14}/></a><a href="https://docs.axiom.trade/llms.txt" target="_blank" rel="noreferrer">Axiom documentation index <ExternalLink size={14}/></a></section></div>}
<footer><span><span className="brand-mark mini"><i/><i/><i/></span>Built for conviction. Grounded in evidence.</span><span><span className="status-dot"/>{logged?'Imported data · Paper execution':'Synthetic preview'}<span className="footer-separator">/</span> Unskilled © 2026</span></footer></main></div>
{toast&&<div className="toast" role="status"><Check size={16}/>{toast}</div>}
{authOpen&&<div className="modal-backdrop" onClick={()=>setAuthOpen(false)}><section className="auth-modal" role="dialog" aria-modal="true" aria-label={register?'Create account':'Sign in'} onClick={e=>e.stopPropagation()}><button className="modal-close icon-button" aria-label="Close sign in" onClick={()=>setAuthOpen(false)}><X size={20}/></button><span className="auth-symbol"><Fingerprint size={30}/></span><span className="overline">YOUR NEXT MOVE STARTS HERE</span><h2>{register?'Make it your workspace.':'Back to the signal.'}</h2><p>{register?'Create an account to build your watchlist.':'Sign in to your Unskilled workspace.'}</p><div className="wallet-signin"><button type="button" className="button" disabled={busy} onClick={()=>walletSignIn('phantom')}>Continue with Phantom</button><button type="button" className="button" disabled={busy} onClick={()=>walletSignIn('metamask')}>Continue with MetaMask</button></div>
<p className="fine-print">Signing proves you control the address. It does not approve a transaction, a transfer or a spending allowance. Either way your trading wallet is created and held here until you close the account.</p>
<div className="auth-divider"><span>or use an email address</span></div><form onSubmit={e=>{e.preventDefault();const f=new FormData(e.currentTarget);run(async()=>{const c={email:f.get('email'),password:f.get('password')};if(register)await api('/auth/register','POST',c);await api('/auth/login','POST',c);await refresh();setAuthOpen(false);setToast(register?'Workspace created':'Welcome back')})}}><label className="field">Email address<input name="email" type="email" autoComplete="email" autoFocus required placeholder="you@example.com"/></label><label className="field">Password<input name="password" type="password" autoComplete={register?'new-password':'current-password'} minLength={register?12:1} maxLength={128} required placeholder={register?'At least 12 characters':'Your password'}/></label>{error&&<p className="form-error" role="alert">{error}</p>}<button className="button primary full" disabled={busy}>{busy?'Working…':register?'Create workspace':'Sign in'}<ArrowRight size={16}/></button></form><p className="auth-switch">{register?'Already have an account?':'New here?'} <button onClick={()=>{setRegister(!register);setError('')}}>{register?'Sign in':'Create an account'}</button></p></section></div>}
{selected&&<div className="drawer-backdrop" onClick={()=>setSelected(null)}><section className="wallet-drawer" role="dialog" aria-modal="true" aria-label="Wallet analysis" onClick={e=>e.stopPropagation()}><div className="drawer-top"><span className="overline">WALLET DOSSIER</span><button className="icon-button" aria-label="Close wallet analysis" onClick={()=>setSelected(null)}><X size={20}/></button></div><span className="avatar avatar-0 large"><Fingerprint size={30}/></span><h2>{short(selected.wallet)}</h2><button className="address-copy" onClick={()=>run(async()=>{await navigator.clipboard.writeText(selected.wallet);setToast('Wallet address copied')})}>{selected.wallet}<Copy size={13}/></button><div className="button-row"><button className="button primary" disabled={busy} onClick={()=>follow(selected.wallet)}>{data.watches.includes(selected.wallet)?<Check size={15}/>:<Plus size={15}/>} {data.watches.includes(selected.wallet)?'Following wallet':'Follow wallet'}</button><span className="score high">{selected.score}<small>/100</small></span></div><div className="dossier-metrics"><div><span>Realized P&L</span><strong className={selected.realized_pnl_sol>=0?'positive':'negative'}>{number(selected.realized_pnl_sol)} SOL</strong></div><div><span>Win rate</span><strong>{selected.win_rate===null?'—':number(selected.win_rate,1)+'%'}</strong></div><div><span>Profit factor</span><strong>{selected.profit_factor===null?'N/A':number(selected.profit_factor)+'×'}</strong></div><div><span>Realized drawdown</span><strong>{number(selected.max_drawdown_sol)} SOL</strong></div></div><LineChart values={selected.curve.map(p=>p[1])}/><div className="rules"><span>Matched sells<strong>{selected.matched_sells}</strong></span><span>Median lot holding time<strong>{selected.median_hold_seconds===null?'—':number(selected.median_hold_seconds/60,0)+' min'}</strong></span><span>Unmatched sell quantity<strong>{number(selected.unmatched_quantity)}</strong></span><span>95% win-rate interval (IID assumption)<strong>{selected.win_rate_interval_95?selected.win_rate_interval_95.map(n=>number(n,1)+'%').join(' – '):'—'}</strong></span><span>Mean P&L per matched sell<strong>{selected.expectancy_sol==null?'—':number(selected.expectancy_sol)+' SOL'}</strong></span><span>Largest win / gross gains<strong>{selected.largest_win_share==null?'—':number(selected.largest_win_share,1)+'%'}</strong></span><span>Longest losing streak<strong>{selected.max_loss_streak??'—'}</strong></span></div>{copy&&<><h3>Copyability <span className="subtle-label">AT {number(copy.order_sol)} SOL</span></h3><p className="fine-print">What a follower would have realized entering and exiting after a delay, at observed prices, at your own order size. This wallet's own realized P&amp;L was {number(copy.leader_realized_pnl_sol)} SOL. No pool impact, queue competition or failed transactions are modeled, so these results are optimistic.</p><div className="table-scroll"><table><thead><tr><th>Delay</th><th>Follower P&amp;L</th><th>Win rate</th><th>Completed</th><th>Not filled</th><th>Entry slippage</th></tr></thead><tbody>{copy.decay.map(d=><tr key={d.delay_seconds}><td>{d.delay_seconds}s</td><td className={d.realized_pnl_sol>=0?'positive':'negative'}>{number(d.realized_pnl_sol)} SOL</td><td>{d.win_rate===null?'—':number(d.win_rate,1)+'%'}</td><td>{d.resolved}/{d.attempts}</td><td>{d.unfilled_entries+d.unknown_liquidity}</td><td>{d.median_entry_slippage_pct===null?'—':number(d.median_entry_slippage_pct,2)+'%'}</td></tr>)}</tbody></table></div>{copy.flags.map(f=><div className="flag" key={f}><CircleHelp size={15}/><span>{f}</span></div>)}</>}{authentic&&<><h3>Authenticity</h3>{authentic.blocked&&<div className="flag"><CircleHelp size={15}/><span>{authentic.blocked}</span></div>}<div className="rules"><span>Entries near a token's first observed trade<strong>{authentic.early_entries}/{authentic.buys}{authentic.early_entry_pct===null?'':' · '+number(authentic.early_entry_pct,0)+'%'}</strong></span><span>Early window<strong>{authentic.early_window_seconds}s</strong></span><span>Observed funders<strong>{authentic.funding_graph_observed?authentic.funders.length:'Unknown'}</strong></span><span>Wallets sharing a funder<strong>{authentic.co_funded_wallets.length}</strong></span><span>Funders that traded first<strong>{authentic.funded_by_counterparty.length}</strong></span></div>{authentic.flags.map(f=><div className="flag" key={f}><CircleHelp size={15}/><span>{f}</span></div>)}</>}<h3>Context & limitations</h3>{selected.flags.map(f=><div className="flag" key={f}><CircleHelp size={15}/><span>{f}</span></div>)}<h3>Trade history <span className="subtle-label">{selected.history.length} events</span></h3><div className="drawer-history">{selected.history.slice().reverse().map(t=><div className="feed-row" key={t.id}><span className={'trade-icon '+t.side}>{t.side==='buy'?<ArrowDownLeft size={16}/>:<ArrowUpRight size={16}/>}</span><div className="feed-main"><strong>{t.symbol} <span>{t.side}</span></strong><small>{date(t.timestamp)} · Fee {number(t.fee_sol,5)} SOL</small></div><strong>{number(t.quantity*t.price_sol)} SOL</strong></div>)}</div></section></div>}
</div>}
function Metric({label,value,unit,foot,icon,positive=false,trend}:{label:string;value:string;unit?:string;foot:string;icon:React.ReactNode;positive?:boolean;trend?:string}){return <section className="metric"><div className="metric-label">{label}{icon}</div><div className={'metric-value '+(positive?'positive':'')}>{value}<small>{unit}</small>{trend&&<span className="metric-trend">{trend}</span>}</div><p>{foot}</p></section>}
function Empty({title,text}:{title:string;text:string}){return <div className="empty"><Crosshair size={28}/><h3>{title}</h3><p>{text}</p></div>}
// A render error anywhere used to unmount the entire application, leaving a
// black page with no way back. That has now happened twice from the same
// cause: a response missing a key the interface assumed was there. A blank
// screen tells the user nothing and tells us nothing either, so a failure is
// caught, named, and left recoverable without losing the session.
class Boundary extends React.Component<{children:React.ReactNode},{message:string|null}>{
 constructor(p:{children:React.ReactNode}){super(p);this.state={message:null}}
 static getDerivedStateFromError(e:unknown){return {message:e instanceof Error?e.message:String(e)}}
 componentDidCatch(e:unknown){console.error('Render failed',e)}
 render(){if(!this.state.message)return this.props.children;
  return <div className="crash" role="alert"><h2>This view failed to render.</h2>
   <p>The rest of the application is unaffected and nothing was sent. If it keeps happening, this message is the detail worth reporting.</p>
   <code>{this.state.message}</code>
   <button className="button primary" onClick={()=>this.setState({message:null})}>Try again</button></div>}}
createRoot(document.getElementById('root')!).render(<React.StrictMode><Boundary><App/></Boundary></React.StrictMode>);

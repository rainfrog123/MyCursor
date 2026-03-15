use crate::machine_id::MachineIdRestorer;
use crate::{log_error, log_info, log_warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::{fs, io};

const MARKER: &str = "/* __MYCURSOR_SEAMLESS__ */";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeamlessStatus {
    pub injected: bool,
    pub server_running: bool,
    pub port: u16,
    pub backup_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SwitchRequest {
    email: String,
}

// ---------------------------------------------------------------------------
// HTTP 服务器
// ---------------------------------------------------------------------------

static SERVER_RUNNING: AtomicBool = AtomicBool::new(false);
static SERVER_PORT: AtomicU16 = AtomicU16::new(36529);
struct ServerHandle {
    stop_flag: Arc<AtomicBool>,
}
fn server_handle() -> &'static Mutex<Option<ServerHandle>> {
    static H: OnceLock<Mutex<Option<ServerHandle>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

fn cors_headers(
    r: tiny_http::Response<io::Cursor<Vec<u8>>>,
) -> tiny_http::Response<io::Cursor<Vec<u8>>> {
    r.with_header("Access-Control-Allow-Origin: *".parse::<tiny_http::Header>().unwrap())
        .with_header("Access-Control-Allow-Methods: GET, POST, OPTIONS".parse::<tiny_http::Header>().unwrap())
        .with_header("Access-Control-Allow-Headers: Content-Type".parse::<tiny_http::Header>().unwrap())
        .with_header("Content-Type: application/json; charset=utf-8".parse::<tiny_http::Header>().unwrap())
}

fn json_resp(status: u16, body: &str) -> tiny_http::Response<io::Cursor<Vec<u8>>> {
    cors_headers(tiny_http::Response::from_string(body).with_status_code(tiny_http::StatusCode(status)))
}

fn read_account_cache() -> Result<serde_json::Value, String> {
    let p = crate::get_data_dir()
        .map_err(|e| e.to_string())?
        .join("account_cache.json");
    if !p.exists() {
        return Ok(serde_json::json!([]));
    }
    let c = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    serde_json::from_str(&c).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortSettings {
    pub field: String,
    pub order: String,
}

impl Default for SortSettings {
    fn default() -> Self {
        Self {
            field: "usage".to_string(),
            order: "desc".to_string(),
        }
    }
}

fn get_sort_settings_path() -> Result<PathBuf, String> {
    crate::get_data_dir()
        .map(|dir| dir.join("sort_settings.json"))
        .map_err(|e| e.to_string())
}

pub fn read_sort_settings() -> SortSettings {
    match get_sort_settings_path() {
        Ok(p) if p.exists() => {
            fs::read_to_string(&p)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default()
        }
        _ => SortSettings::default(),
    }
}

pub fn save_sort_settings(field: &str, order: &str) -> Result<(), String> {
    let p = get_sort_settings_path()?;
    let settings = SortSettings {
        field: field.to_string(),
        order: order.to_string(),
    };
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| e.to_string())?;
    log_info!("[排序设置] 已保存: field={}, order={}", field, order);
    Ok(())
}

fn handle_accounts() -> tiny_http::Response<io::Cursor<Vec<u8>>> {
    match read_account_cache() {
        Ok(a) => {
            // Filter out stashed accounts (accounts with "stashed" tag)
            let filtered: Vec<serde_json::Value> = a
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter(|acc| {
                            !acc.get("tags")
                                .and_then(|t| t.as_array())
                                .map(|tags| tags.iter().any(|t| t.as_str() == Some("stashed")))
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            let sort = read_sort_settings();
            json_resp(200, &serde_json::json!({
                "code": 0,
                "data": filtered,
                "sort": {
                    "field": sort.field,
                    "order": sort.order
                }
            }).to_string())
        }
        Err(e) => json_resp(500, &serde_json::json!({"code":1,"msg":e}).to_string()),
    }
}

fn handle_switch(body: &str) -> tiny_http::Response<io::Cursor<Vec<u8>>> {
    let req: SwitchRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return json_resp(
                400,
                &serde_json::json!({"code":1,"msg":e.to_string()}).to_string(),
            )
        }
    };
    let accs = match read_account_cache() {
        Ok(a) => a,
        Err(e) => return json_resp(500, &serde_json::json!({"code":1,"msg":e}).to_string()),
    };
    match accs
        .as_array()
        .and_then(|a| a.iter().find(|x| x["email"].as_str() == Some(&req.email)))
    {
        Some(acc) => {
            log_info!("[无感换号] 切换: {}", req.email);
            json_resp(
                200,
                &serde_json::json!({
                    "code": 0,
                    "data": {
                        "token": acc["token"],
                        "email": acc["email"],
                        "refresh_token": acc["refresh_token"],
                        "machine_ids": acc.get("machine_ids")
                    }
                })
                .to_string(),
            )
        }
        None => json_resp(
            404,
            &serde_json::json!({"code":1,"msg":"未找到"}).to_string(),
        ),
    }
}

fn run_server(port: u16, stop: Arc<AtomicBool>) {
    let addr = format!("127.0.0.1:{}", port);
    let srv = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            log_error!("[无感换号] 启动失败: {}", e);
            SERVER_RUNNING.store(false, Ordering::SeqCst);
            return;
        }
    };
    log_info!("[无感换号] 服务器启动: {}", addr);
    SERVER_RUNNING.store(true, Ordering::SeqCst);
    SERVER_PORT.store(port, Ordering::SeqCst);

    while !stop.load(Ordering::SeqCst) {
        match srv.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(Some(mut req)) => {
                let u = req.url().to_string();
                let m = req.method().to_string();
                if m == "OPTIONS" {
                    let _ = req.respond(json_resp(200, "{}"));
                    continue;
                }
                let r = match (m.as_str(), u.as_str()) {
                    ("GET", "/api/health") => json_resp(200, r#"{"status":"ok"}"#),
                    ("GET", "/api/accounts") => handle_accounts(),
                    ("POST", "/api/switch") => {
                        let mut b = String::new();
                        let _ = req.as_reader().read_to_string(&mut b);
                        handle_switch(&b)
                    }
                    _ => json_resp(404, r#"{"code":1}"#),
                };
                let _ = req.respond(r);
            }
            Ok(None) => {}
            Err(e) => {
                log_error!("[无感换号] {}", e);
            }
        }
    }
    SERVER_RUNNING.store(false, Ordering::SeqCst);
}

pub fn start_server(port: u16) -> Result<(), String> {
    let mut h = server_handle().lock().map_err(|e| e.to_string())?;
    if h.is_some() {
        return Err("服务器已在运行".to_string());
    }
    let f = Arc::new(AtomicBool::new(false));
    let fc = f.clone();
    thread::spawn(move || run_server(port, fc));
    thread::sleep(std::time::Duration::from_millis(200));
    *h = Some(ServerHandle { stop_flag: f });
    Ok(())
}

pub fn stop_server() -> Result<(), String> {
    let mut h = server_handle().lock().map_err(|e| e.to_string())?;
    match h.take() {
        Some(s) => {
            s.stop_flag.store(true, Ordering::SeqCst);
            thread::sleep(std::time::Duration::from_millis(600));
            Ok(())
        }
        None => Err("服务器未运行".to_string()),
    }
}

pub fn is_server_running() -> bool {
    SERVER_RUNNING.load(Ordering::SeqCst)
}
pub fn get_server_port() -> u16 {
    SERVER_PORT.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// 注入逻辑
// ---------------------------------------------------------------------------

fn get_backup_path(p: &PathBuf) -> PathBuf {
    let mut b = p.clone();
    let f = b.file_name().unwrap().to_string_lossy().to_string();
    b.set_file_name(format!("{}.backup.seamless", f));
    b
}

/// 生成追加到文件末尾的无感换号脚本
fn make_tail_script(port: u16) -> String {
    format!(
        r##"
{marker}
;(function(){{
"use strict";
var PORT={port};
/* 安全清空子元素（Cursor 启用了 Trusted Types，禁止 innerHTML） */
function clr(el){{while(el.firstChild)el.removeChild(el.firstChild)}}
/* 样式 */
function mcS(){{if(document.getElementById('mc-st'))return;var s=document.createElement('style');s.id='mc-st';s.textContent='@keyframes mcFI{{from{{opacity:0;transform:translateX(40px)}}to{{opacity:1;transform:translateX(0)}}}}@keyframes mcSU{{from{{opacity:0;transform:translateY(16px)}}to{{transform:translateY(0);opacity:1}}}}#mc-btn{{transition:opacity .2s,box-shadow .2s}}#mc-btn:hover{{opacity:1!important;box-shadow:0 4px 20px rgba(0,0,0,.5)!important}}#mc-btn.mc-drag{{opacity:1!important;box-shadow:0 0 0 2px #4fc3f7!important;cursor:grabbing!important}}';document.head.appendChild(s)}}
/* 通知 */
function notif(m,c){{try{{var d=document.getElementById('mc-n')||function(){{var d=document.createElement('div');d.id='mc-n';d.style.cssText='position:fixed;top:20px;right:20px;z-index:999999;display:flex;flex-direction:column;gap:8px;max-width:420px;pointer-events:auto';document.body.appendChild(d);return d}}();var n=document.createElement('div');n.style.cssText='background:#1e1e1e;border:1px solid #333;border-left:3px solid '+c+';padding:12px 16px;border-radius:8px;box-shadow:0 4px 16px rgba(0,0,0,.5);color:#ccc;font-size:13px;display:flex;align-items:center;gap:10px;animation:mcFI .3s ease';var ic=document.createElement('span');ic.style.cssText='font-size:16px;flex-shrink:0';ic.textContent=c==='#4ec9b0'?'\u2713':'\u2717';n.appendChild(ic);var tx=document.createElement('span');tx.textContent=m;n.appendChild(tx);d.appendChild(n);setTimeout(function(){{n.style.opacity='0';n.style.transition='opacity .3s';setTimeout(function(){{n.remove();if(d.children.length===0)d.remove()}},300)}},5000)}}catch(e){{}}}}
/* 订阅信息映射 */
var SL={{'free':{{t:'Free',c:'#4a89dc',b:'rgba(74,137,220,.12)'}},'pro':{{t:'Pro',c:'#a855f7',b:'rgba(168,85,247,.15)'}},'pro_plus':{{t:'Pro+',c:'#a855f7',b:'rgba(168,85,247,.15)'}},'ultra':{{t:'Ultra',c:'#f59e0b',b:'rgba(245,158,11,.15)'}},'token_expired':{{t:'\u5df2\u5931\u6548',c:'#f48771',b:'rgba(244,135,113,.15)'}}}};
function SI(s){{return SL[s]||{{t:s||'\u672a\u77e5',c:'#888',b:'rgba(136,136,136,.12)'}}}}
/* 排序函数（使用API返回的设置） */
function sortAccs(arr,sf,so){{
if(!sf||sf==='none')return arr;
var subOrd={{'ultra':1,'pro_plus':2,'pro':3,'business':4,'free_trial':5,'free':6,'token_expired':7}};
return arr.slice().sort(function(a,b){{
var cmp=0;
if(sf==='date'){{var da=new Date(a.created_at||0).getTime(),db=new Date(b.created_at||0).getTime();cmp=da-db}}
else if(sf==='trial'){{var ta=a.trial_days_remaining!=null?a.trial_days_remaining:-1,tb=b.trial_days_remaining!=null?b.trial_days_remaining:-1;cmp=ta-tb}}
else if(sf==='usage'){{var ua=a.usage_cost_cents||0,ub=b.usage_cost_cents||0;cmp=ua-ub}}
else if(sf==='email'){{cmp=(a.email||'').localeCompare(b.email||'')}}
else if(sf==='subscription'){{var sa=subOrd[a.subscription_type||'free']||99,sb=subOrd[b.subscription_type||'free']||99;cmp=sa-sb}}
return so==='asc'?cmp:-cmp;
}});
}}
/* 浮动按钮（可拖动） */
function mkBtn(){{if(document.getElementById('mc-btn'))return;
var b=document.createElement('div');b.id='mc-btn';
var pos;try{{pos=JSON.parse(localStorage.getItem('mc-btn-pos'))}}catch(e){{}}
var ix=pos&&pos.r!=null?pos.r:20,iy=pos&&pos.b!=null?pos.b:12;
b.style.cssText='position:fixed;bottom:'+iy+'px;right:'+ix+'px;z-index:999998;width:26px;height:26px;border-radius:50%;background:linear-gradient(135deg,#0e639c,#1177bb);color:#fff;display:flex;align-items:center;justify-content:center;cursor:pointer;box-shadow:0 2px 8px rgba(0,0,0,.35);font-size:12px;user-select:none;opacity:.5';
b.textContent='\u26a1';b.title='\u70b9\u51fb\u5207\u6362\u8d26\u53f7\uff0c\u62d6\u52a8\u8c03\u6574\u4f4d\u7f6e';
var dragged=false,startX,startY,startR,startB;
b.addEventListener('mousedown',function(e){{e.preventDefault();dragged=false;startX=e.clientX;startY=e.clientY;startR=parseInt(b.style.right)||0;startB=parseInt(b.style.bottom)||0;b.classList.add('mc-drag');
function onMove(e){{var dx=e.clientX-startX,dy=e.clientY-startY;if(!dragged&&(Math.abs(dx)>3||Math.abs(dy)>3))dragged=true;if(dragged){{b.style.right=Math.max(0,Math.min(window.innerWidth-30,startR-dx))+'px';b.style.bottom=Math.max(0,Math.min(window.innerHeight-30,startB-dy))+'px'}}}}
function onUp(){{document.removeEventListener('mousemove',onMove);document.removeEventListener('mouseup',onUp);b.classList.remove('mc-drag');if(dragged){{try{{localStorage.setItem('mc-btn-pos',JSON.stringify({{r:parseInt(b.style.right),b:parseInt(b.style.bottom)}}))}}catch(e){{}}}}else{{fetchPick()}}}}
document.addEventListener('mousemove',onMove);document.addEventListener('mouseup',onUp)}});
document.body.appendChild(b)}}
/* 获取账号并弹窗 */
function fetchPick(){{
(async function(){{try{{
var r=await fetch('http://127.0.0.1:'+PORT+'/api/accounts');
var d=await r.json();
if(d.code===0&&d.data&&d.data.length){{
var sf=d.sort&&d.sort.field||'usage',so=d.sort&&d.sort.order||'desc';
pick(sortAccs(d.data,sf,so));
}}else notif('\u6ca1\u6709\u53ef\u7528\u8d26\u53f7','#f48771');
}}catch(e){{notif('\u8fde\u63a5\u5931\u8d25: '+(e.message||e),'#f48771');console.error('[MyCursor] fetch error',e)}}}})()}}
/* 创建筛选药丸 */
function mkP(txt,act,fn){{var p=document.createElement('span');p.textContent=txt;p.style.cssText='padding:2px 10px;border-radius:12px;font-size:11px;cursor:pointer;transition:all .15s;user-select:none;white-space:nowrap;'+(act?'background:rgba(14,99,156,.3);color:#4fc3f7;border:1px solid rgba(14,99,156,.5)':'background:transparent;color:#888;border:1px solid #3c3c3c');p.onmouseover=function(){{if(!act)p.style.background='#2a2d2e'}};p.onmouseout=function(){{if(!act)p.style.background=act?'rgba(14,99,156,.3)':'transparent'}};p.onclick=fn;return p}}
/* 账号选择弹窗（含筛选） */
function pick(accs){{
var old=document.getElementById('mc-pick');if(old)old.remove();
var subMap={{}},tagMap={{}};
accs.forEach(function(a){{var s=a.subscription_type||'unknown';subMap[s]=(subMap[s]||0)+1;(a.tags||[]).forEach(function(t){{tagMap[t]=(tagMap[t]||0)+1}})}});
var subKeys=Object.keys(subMap),tagKeys=Object.keys(tagMap);
var fS='all',fT='';
function getF(){{return accs.filter(function(a){{if(fS!=='all'){{var s=a.subscription_type||'unknown';if(s!==fS)return false}}if(fT){{if(!a.tags||a.tags.indexOf(fT)===-1)return false}}return true}})}}
/* 遮罩 */
var o=document.createElement('div');o.id='mc-pick';o.style.cssText='position:fixed;inset:0;background:rgba(0,0,0,.5);z-index:999999;display:flex;align-items:center;justify-content:center;backdrop-filter:blur(3px)';
var modal=document.createElement('div');modal.style.cssText='background:#1e1e1e;border:1px solid #3c3c3c;border-radius:12px;max-width:520px;width:92%;max-height:80vh;display:flex;flex-direction:column;box-shadow:0 12px 40px rgba(0,0,0,.6);animation:mcSU .25s ease';
/* 标题栏 */
var hdr=document.createElement('div');hdr.style.cssText='padding:14px 20px;border-bottom:1px solid #2d2d2d;display:flex;align-items:center;justify-content:space-between';
var tl=document.createElement('div');tl.style.cssText='display:flex;align-items:center;gap:10px';
var tic=document.createElement('span');tic.style.cssText='font-size:18px';tic.textContent='\u26a1';
var tt=document.createElement('span');tt.style.cssText='color:#e0e0e0;font-size:15px;font-weight:600';tt.textContent='\u5207\u6362\u8d26\u53f7';
tl.appendChild(tic);tl.appendChild(tt);
var cnt=document.createElement('span');cnt.style.cssText='color:#666;font-size:12px;margin-left:8px';tl.appendChild(cnt);
var xb=document.createElement('button');xb.textContent='\u2715';xb.style.cssText='background:none;border:none;color:#666;font-size:18px;cursor:pointer;padding:4px 8px;border-radius:6px;transition:all .15s';
xb.onmouseover=function(){{xb.style.color='#ccc';xb.style.background='#333'}};xb.onmouseout=function(){{xb.style.color='#666';xb.style.background='none'}};
xb.onclick=function(){{o.remove()}};
hdr.appendChild(tl);hdr.appendChild(xb);modal.appendChild(hdr);
/* 筛选栏 */
var fb=document.createElement('div');fb.style.cssText='padding:10px 16px;border-bottom:1px solid #2d2d2d;display:flex;flex-direction:column;gap:6px';
var sr=document.createElement('div');sr.style.cssText='display:flex;flex-wrap:wrap;gap:4px;align-items:center';
var trow=document.createElement('div');trow.style.cssText='display:flex;flex-wrap:wrap;gap:4px;align-items:center';
function rflt(){{
clr(sr);
var sl=document.createElement('span');sl.style.cssText='font-size:11px;color:#555;margin-right:2px';sl.textContent='\u7c7b\u578b:';sr.appendChild(sl);
sr.appendChild(mkP('\u5168\u90e8 ('+accs.length+')',fS==='all',function(){{fS='all';rflt();rList()}}));
subKeys.forEach(function(k){{var i=SI(k);sr.appendChild(mkP(i.t+' ('+subMap[k]+')',fS===k,function(){{fS=k;rflt();rList()}}))}});
if(tagKeys.length>0){{clr(trow);var tl2=document.createElement('span');tl2.style.cssText='font-size:11px;color:#555;margin-right:2px';tl2.textContent='\u6807\u7b7e:';trow.appendChild(tl2);
trow.appendChild(mkP('\u5168\u90e8',fT==='',function(){{fT='';rflt();rList()}}));
tagKeys.forEach(function(t){{trow.appendChild(mkP(t+' ('+tagMap[t]+')',fT===t,function(){{fT=t;rflt();rList()}}))}})}}
}}
rflt();fb.appendChild(sr);if(tagKeys.length>0)fb.appendChild(trow);modal.appendChild(fb);
/* 列表 */
var ls=document.createElement('div');ls.style.cssText='padding:8px;overflow-y:auto;flex:1';
function rList(){{clr(ls);var fl=getF();
cnt.textContent=fl.length===accs.length?('\u5171 '+accs.length+' \u4e2a'):('\u663e\u793a '+fl.length+' / '+accs.length);
if(fl.length===0){{var emp=document.createElement('div');emp.style.cssText='padding:32px;text-align:center;color:#555;font-size:13px';emp.textContent='\u6ca1\u6709\u5339\u914d\u7684\u8d26\u53f7';ls.appendChild(emp);return}}
fl.forEach(function(a){{
var it=document.createElement('div');it.style.cssText='padding:10px 14px;margin:2px 0;border-radius:8px;cursor:pointer;display:flex;align-items:center;gap:12px;transition:background .12s';
it.onmouseover=function(){{it.style.background='#2a2d2e'}};it.onmouseout=function(){{it.style.background='transparent'}};
var av=document.createElement('div');av.style.cssText='width:34px;height:34px;border-radius:50%;background:#0e639c;color:#fff;display:flex;align-items:center;justify-content:center;font-size:14px;font-weight:600;flex-shrink:0';
av.textContent=(a.email||'?')[0].toUpperCase();
var inf=document.createElement('div');inf.style.cssText='flex:1;min-width:0';
var em=document.createElement('div');em.style.cssText='color:#ccc;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;display:flex;align-items:center;gap:6px';
var emT=document.createElement('span');emT.style.cssText='font-family:monospace;overflow:hidden;text-overflow:ellipsis';emT.textContent=a.email||'';em.appendChild(emT);
if(a.is_current){{var cur=document.createElement('span');cur.style.cssText='padding:1px 6px;border-radius:8px;font-size:10px;background:rgba(78,201,176,.15);color:#4ec9b0;flex-shrink:0';cur.textContent='\u5f53\u524d';em.appendChild(cur)}}
inf.appendChild(em);
var badges=document.createElement('div');badges.style.cssText='display:flex;flex-wrap:wrap;gap:4px;margin-top:4px';
var sub=a.subscription_type;
if(sub){{var si=SI(sub);var subBadge=document.createElement('span');subBadge.style.cssText='display:inline-block;padding:1px 8px;border-radius:10px;font-size:10px;font-weight:500;background:'+si.b+';color:'+si.c;subBadge.textContent=si.t;badges.appendChild(subBadge)}}
(a.tags||[]).forEach(function(t){{var tb=document.createElement('span');tb.style.cssText='display:inline-block;padding:1px 8px;border-radius:10px;font-size:10px;background:rgba(78,201,176,.12);color:#4ec9b0';tb.textContent=t;badges.appendChild(tb)}});
if(badges.childNodes.length)inf.appendChild(badges);
it.appendChild(av);it.appendChild(inf);
it.onclick=function(){{o.remove();doSwitch(a)}};
ls.appendChild(it);
}})}}
rList();modal.appendChild(ls);
o.appendChild(modal);
o.onclick=function(e){{if(e.target===o)o.remove()}};
document.addEventListener('keydown',function esc(e){{if(e.key==='Escape'){{o.remove();document.removeEventListener('keydown',esc)}}}});
document.body.appendChild(o);
}}
/* 切换 */
function doSwitch(a){{
(async function(){{try{{
var r=await fetch('http://127.0.0.1:'+PORT+'/api/switch',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{email:a.email}})}});
var d=await r.json();
if(d.code===0&&d.data&&d.data.token){{
var auth=window.__mcAuthService;
if(auth&&'localOverrideAccessToken' in auth){{
auth.localOverrideAccessToken=d.data.token;
if(d.data.refresh_token)auth.refreshToken=function(){{return d.data.refresh_token}};
if(!auth.__mcPatched){{auth.__mcPatched=true;auth.storeAccessRefreshToken=function(r,s){{auth.localOverrideAccessToken=r}}}}
if(d.data.machine_ids){{var mi=d.data.machine_ids;if(mi['telemetry.machineId'])auth._machineId=mi['telemetry.machineId'];if(mi['telemetry.macMachineId'])auth._macMachineId=mi['telemetry.macMachineId']}}
auth.notifyLoginChangedListeners(true);
var newEmail=d.data.email||a.email;
window.__mcCurrentEmail=newEmail;
var emailEl=document.querySelector('.cursor-settings-sidebar-header-email');
if(emailEl)emailEl.textContent=newEmail;
window.__mcLastSwitch=Date.now();
notif('\u5df2\u5207\u6362\u5230 '+newEmail+'\uff0c\u8bf7\u91cd\u65b0\u53d1\u9001','#4ec9b0');
}}else{{notif('Auth \u672a\u5c31\u7eea\uff0c\u8bf7\u91cd\u542f Cursor','#f48771')}}
}}else{{notif(d.msg||'\u5207\u6362\u5931\u8d25','#f48771')}}
}}catch(err){{notif('\u5207\u6362\u5931\u8d25: '+(err.message||err),'#f48771')}}}})()}}
/* 邮箱显示同步 */
function watchEmail(){{
new MutationObserver(function(){{
if(!window.__mcCurrentEmail)return;
var el=document.querySelector('.cursor-settings-sidebar-header-email');
if(el&&el.textContent!==window.__mcCurrentEmail)el.textContent=window.__mcCurrentEmail;
}}).observe(document.body,{{childList:true,subtree:true}});
}}
function boot(){{if(document.body){{mcS();mkBtn();watchEmail()}}else setTimeout(boot,300)}}
boot();
}})();
"##,
        marker = MARKER,
        port = port,
    )
}

/// 执行注入
pub fn inject_seamless(port: u16) -> Result<serde_json::Value, String> {
    let wp = MachineIdRestorer::get_workbench_js_path().map_err(|e| e.to_string())?;
    if !wp.exists() {
        return Err(format!("文件不存在: {}", wp.display()));
    }
    let bp = get_backup_path(&wp);
    let mut det = Vec::new();

    // 备份保护
    if bp.exists() {
        let bc = fs::read_to_string(&bp).unwrap_or_default();
        if bc.contains("__MYCURSOR_SEAMLESS__") {
            let cc = fs::read_to_string(&wp).unwrap_or_default();
            if !cc.contains("__MYCURSOR_SEAMLESS__") {
                fs::copy(&wp, &bp).map_err(|e| e.to_string())?;
                det.push("备份已污染，已用干净文件重建".to_string());
            } else {
                return Err("备份和当前文件都已注入，请手动恢复".to_string());
            }
        } else {
            fs::copy(&bp, &wp).map_err(|e| e.to_string())?;
            det.push("已从备份恢复".to_string());
        }
    } else {
        let cc = fs::read_to_string(&wp).unwrap_or_default();
        if cc.contains("__MYCURSOR_SEAMLESS__") {
            return Err("已注入但无备份，请手动恢复".to_string());
        }
        fs::copy(&wp, &bp).map_err(|e| e.to_string())?;
        det.push(format!("已创建备份: {}", bp.display()));
    }

    let mut content = fs::read_to_string(&wp).map_err(|e| e.to_string())?;
    let orig_len = content.len();

    // 步骤 1: 绕过完整性检查
    let t1 = "_showNotification(){";
    if content.contains(t1) {
        content = content.replacen(t1, &format!("_showNotification(){{{}", MARKER), 1);
        det.push("步骤1: 完整性检查绕过 OK".to_string());
    } else {
        det.push("步骤1: 未找到 _showNotification".to_string());
    }

    // 步骤 2: hook addLoginChangedListener 捕获 auth service
    let t2 = "addLoginChangedListener(e){this.loginChangedListeners.push(e)}";
    if content.contains(t2) {
        let replacement = format!(
            "addLoginChangedListener(e){{this.loginChangedListeners.push(e);window.__mcAuthService=this}}"
        );
        content = content.replacen(t2, &replacement, 1);
        det.push("步骤2: Auth Service 钩子 OK".to_string());
    } else {
        det.push("步骤2: 未找到 addLoginChangedListener（关键！）".to_string());
    }

    // 步骤 3: 追加尾部脚本
    let script = make_tail_script(port);
    content.push_str(&script);
    det.push("步骤3: 监听脚本已追加".to_string());

    fs::write(&wp, &content).map_err(|e| e.to_string())?;

    // 更新 product.json 校验和
    match MachineIdRestorer::new() {
        Ok(restorer) => match restorer.update_product_json_checksums() {
            Ok(()) => det.push("product.json 校验和已更新".to_string()),
            Err(e) => {
                log_warn!("[无感换号] 更新校验和失败: {}", e);
                det.push(format!("Warning: 校验和更新失败: {}", e));
            }
        },
        Err(e) => det.push(format!("Warning: 初始化失败: {}", e)),
    }

    log_info!(
        "[无感换号] 注入完成: {} -> {} 字节",
        orig_len,
        content.len()
    );

    Ok(serde_json::json!({
        "success": true,
        "message": format!("注入成功 (端口 {})", port),
        "details": det,
        "port": port
    }))
}

/// 恢复原始文件
pub fn restore_seamless() -> Result<serde_json::Value, String> {
    let wp = MachineIdRestorer::get_workbench_js_path().map_err(|e| e.to_string())?;
    let bp = get_backup_path(&wp);
    if !bp.exists() {
        return Ok(serde_json::json!({"success":false,"message":"无备份"}));
    }
    fs::copy(&bp, &wp).map_err(|e| e.to_string())?;

    if let Ok(restorer) = MachineIdRestorer::new() {
        if let Err(e) = restorer.update_product_json_checksums() {
            log_warn!("[无感换号] 恢复后更新校验和失败: {}", e);
        }
    }

    log_info!("[无感换号] 已恢复");
    Ok(serde_json::json!({"success":true,"message":"已恢复，请重启 Cursor"}))
}

/// 查询状态
pub fn get_seamless_status() -> Result<SeamlessStatus, String> {
    let wp = MachineIdRestorer::get_workbench_js_path().map_err(|e| e.to_string())?;
    let bp = get_backup_path(&wp);
    let injected = if wp.exists() {
        let c = fs::read_to_string(&wp).map_err(|e| e.to_string())?;
        c.contains("__MYCURSOR_SEAMLESS__")
    } else {
        false
    };
    Ok(SeamlessStatus {
        injected,
        server_running: is_server_running(),
        port: get_server_port(),
        backup_exists: bp.exists(),
    })
}

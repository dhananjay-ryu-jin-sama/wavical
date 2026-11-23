window.tailwind = window.tailwind || {};
tailwind.config = {
  theme: {
    extend: {
      colors: {
        ink: {950:'#04060c',900:'#070b14',800:'#0b1120',700:'#101a2e',600:'#16233d'},
        teal2:'#2dd4bf', sky2:'#38bdf8', gold2:'#fbbf24'
      },
      fontFamily: {
        display:['Syne','sans-serif'],
        body:['"Space Grotesk"','sans-serif'],
        mono:['"JetBrains Mono"','monospace']
      }
    }
  }
}

document.addEventListener('error', function(event){
  if(event.target.matches && event.target.matches('img[data-remove-on-error]')) event.target.remove();
}, true);

document.addEventListener('DOMContentLoaded', function(){
/* ============================================================
   GAURA NITAI TECHNOLOGIES — Interactive Engine
============================================================ */
(function(){
'use strict';

/* ---------- Hard fallback if CDN libs fail ---------- */
function hardReveal(){
  var pre=document.getElementById('preloader'); if(pre) pre.style.display='none';
  document.body.classList.remove('locked');
  var nav=document.getElementById('site-nav'); if(nav) nav.style.opacity='1';
  var t=document.getElementById('tagline-text'); if(t && !t.textContent) t.textContent='Engineering intelligent, connected & secure products for the world of tomorrow.';
}
window.addEventListener('load',function(){ setTimeout(function(){ if(!window.gsap) hardReveal(); },400); });
setTimeout(function(){ if(!window.gsap) hardReveal(); },6000);
if(!window.gsap) return;

gsap.registerPlugin(ScrollTrigger);
var isFine = window.matchMedia('(pointer:fine)').matches;
var reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* ---------- Smooth scroll (Lenis) ---------- */
var lenis=null;
if(window.Lenis && !reduced){
  lenis=new Lenis({
    duration:0.8,
    easing:function(t){ return Math.min(1, 1.001 - Math.pow(2, -10 * t)); },
    smoothWheel:true,
    wheelMultiplier:1.15,
    touchMultiplier:1.5,
    syncTouch:false
  });
  lenis.on('scroll',ScrollTrigger.update);
  gsap.ticker.add(function(t){ lenis.raf(t*1000); });
  gsap.ticker.lagSmoothing(500, 33);
  lenis.stop();
}

/* ---------- Build the lotus petals ---------- */
var NS='http://www.w3.org/2000/svg';
var PETALS=[
  {d:'M200 250 C184 208 178 150 200 96 C222 150 216 208 200 250 Z', cls:'petal petal-outer', angles:[-78,-52,-26,0,26,52,78]},
  {d:'M200 250 C187 214 183 166 200 122 C217 166 213 214 200 250 Z', cls:'petal petal-mid',   angles:[-60,-30,0,30,60]},
  {d:'M200 250 C190 222 187 186 200 152 C213 186 210 222 200 250 Z', cls:'petal petal-inner', angles:[-30,0,30]},
  {d:'M200 250 C193 228 191 200 200 172 C209 200 207 228 200 250 Z', cls:'petal petal-bud',   angles:[0]}
];
var petalsGroup=document.getElementById('lotus-petals');
PETALS.forEach(function(layer){
  layer.angles.forEach(function(a){
    var p=document.createElementNS(NS,'path');
    p.setAttribute('d',layer.d);
    p.setAttribute('class',layer.cls);
    p.dataset.angle=a;
    petalsGroup.appendChild(p);
  });
});
var petalPaths=Array.prototype.slice.call(petalsGroup.querySelectorAll('path'));
petalPaths.forEach(function(p){ var L=p.getTotalLength(); p.style.strokeDasharray=L; p.style.strokeDashoffset=L; });
var swirlPaths=Array.prototype.slice.call(document.querySelectorAll('.swirl path'));
swirlPaths.forEach(function(p){ var L=p.getTotalLength(); p.style.strokeDasharray=L; p.style.strokeDashoffset=L; p.style.opacity=1; });

/* ---------- Split hero title into letters ---------- */
var titleEl=document.getElementById('hero-title');
var titleText=titleEl.textContent.trim();
titleEl.textContent='';
Array.prototype.forEach.call(titleText,function(ch){
  var s=document.createElement('span');
  if(ch===' '){ s.className='gn-space'; s.innerHTML='&nbsp;'; }
  else{ s.className='gn-letter'; s.textContent=ch; }
  titleEl.appendChild(s);
});

/* ---------- Hero reveal timeline (paused) ---------- */
var TAGLINE='Engineering intelligent, connected & secure products for the world of tomorrow.';
var taglineDone=false;
function typeTagline(){
  if(taglineDone) return; taglineDone=true;
  var el=document.getElementById('tagline-text'); var i=0;
  var iv=setInterval(function(){
    el.textContent=TAGLINE.slice(0,++i);
    if(i>=TAGLINE.length) clearInterval(iv);
  },22);
}
var heroTl=gsap.timeline({paused:true});
heroTl
  .fromTo('#hero-badge',{scale:0,opacity:0},{scale:1,opacity:1,duration:.7,ease:'back.out(1.7)'})
  .fromTo('.hero-kicker',{opacity:0,y:14},{opacity:1,y:0,duration:.6},'-=0.35')
  .fromTo('.gn-letter',{yPercent:120,opacity:0,rotateX:-75},{yPercent:0,opacity:1,rotateX:0,duration:.85,stagger:.032,ease:'power4.out'},'-=0.3')
  .fromTo('#hero-underline',{scaleX:0},{scaleX:1,duration:.85,ease:'power3.inOut'},'-=0.35')
  .add(typeTagline,'-=0.15')
  .fromTo('.hero-cta',{y:34,opacity:0},{y:0,opacity:1,duration:.7,stagger:.12,ease:'power3.out'},'+=0.55')
  .fromTo('#site-nav',{y:-70,opacity:0},{y:0,opacity:1,duration:.7,ease:'power3.out'},'-=0.55')
  .fromTo('#scroll-hint',{opacity:0},{opacity:1,duration:.6},'-=0.2');

/* ---------- Intro timeline ---------- */
var introDone=false, breatheTween=null;
gsap.set(petalPaths,{scale:.15,rotation:0,opacity:0,svgOrigin:'200 250'});

function startBreathe(){
  breatheTween=gsap.to('#lotus-glow',{scale:1.2,repeat:-1,yoyo:true,duration:1.1,ease:'sine.inOut',transformOrigin:'50% 50%'});
}
function unlockPage(){
  document.body.classList.remove('locked');
  if(lenis) lenis.start();
}
function finishIntro(instant){
  if(introDone) return; introDone=true;
  if(breatheTween) breatheTween.kill();
  var pre=document.getElementById('preloader');
  pre.style.display='none';
  unlockPage();
  if(instant){
    heroTl.progress(1);
    taglineDone=true;
    document.getElementById('tagline-text').textContent=TAGLINE;
  }
  setTimeout(function(){ ScrollTrigger.refresh(); buildConnector(); },120);
}
var intro=gsap.timeline({defaults:{ease:'power2.out'}});
intro
  .to(petalPaths,{strokeDashoffset:0,duration:1.45,stagger:.035,ease:'power2.inOut'},0.15)
  .to(petalPaths,{rotation:function(i,el){return +el.dataset.angle;},scale:1,opacity:1,duration:1.55,stagger:.035,ease:'back.out(1.4)',svgOrigin:'200 250'},0.15)
  .to(swirlPaths,{strokeDashoffset:0,duration:.9,stagger:.12,ease:'power2.inOut'},1.1)
  .to('.lotus-dots circle',{opacity:1,duration:.5,stagger:.08},1.35)
  .fromTo('#lotus-glow',{scale:0,opacity:0,transformOrigin:'50% 50%'},{scale:1,opacity:1,duration:.85,ease:'elastic.out(1,0.6)'},1.45)
  .fromTo('#pre-logo-badge',{scale:0,rotation:-25,opacity:0},{scale:1,rotation:0,opacity:1,duration:.85,ease:'back.out(1.8)'},1.75)
  .fromTo('#ring-burst',{scale:.3,opacity:.9},{scale:2.5,opacity:0,duration:1,ease:'power2.out'},1.8)
  .fromTo('#pre-flash',{opacity:0},{opacity:.7,duration:.13,yoyo:true,repeat:1},1.78)
  .add(startBreathe,2.65)
  .to('#lotus-svg',{opacity:0,scale:1.14,duration:.6,ease:'power2.in'},3.35)
  .to('#pre-logo-badge',{y:-60,scale:.5,opacity:0,duration:.55,ease:'power2.in'},3.35)
  .add(function(){ unlockPage(); heroTl.play(); },3.62)
  .to('#preloader',{opacity:0,duration:.7,ease:'power2.inOut'},3.62)
  .add(function(){ finishIntro(false); },4.4);

document.getElementById('skip-intro').addEventListener('click',function(){
  if(introDone) return;
  intro.progress(1);
  finishIntro(true);
});
if(reduced){ intro.progress(1); finishIntro(true); }

/* ---------- Hero constellation canvas ---------- */
var canvas=document.getElementById('hero-canvas'), ctx=canvas.getContext('2d');
var W=0,H=0,parts=[],mouse={x:-9999,y:-9999},canvasRunning=true,canvasRafId=null;
var COLORS=['rgba(45,212,191,','rgba(56,189,248,','rgba(251,191,36,'];
function sizeCanvas(){
  var dpr=Math.min(1.5,window.devicePixelRatio||1);
  W=canvas.offsetWidth; H=canvas.offsetHeight;
  canvas.width=W*dpr; canvas.height=H*dpr;
  ctx.setTransform(dpr,0,0,dpr,0,0);
  var n=Math.min(48,Math.floor(W*H/28000));
  parts=[];
  for(var i=0;i<n;i++){
    parts.push({x:Math.random()*W,y:Math.random()*H,vx:(Math.random()-.5)*.35,vy:(Math.random()-.5)*.35,r:Math.random()*1.8+.7,c:COLORS[i%3],a:Math.random()*.5+.3});
  }
}
function drawCanvas(){
  if(!canvasRunning){ canvasRafId=null; return; }
  ctx.clearRect(0,0,W,H);
  var i,j,p,q,dx,dy,d;
  for(i=0;i<parts.length;i++){
    p=parts[i];
    p.x+=p.vx; p.y+=p.vy;
    dx=p.x-mouse.x; dy=p.y-mouse.y; d=dx*dx+dy*dy;
    if(d<16900 && d>0.1){ var dist=Math.sqrt(d); p.x+=dx/dist*.9; p.y+=dy/dist*.9; }
    if(p.x<-20)p.x=W+20; if(p.x>W+20)p.x=-20;
    if(p.y<-20)p.y=H+20; if(p.y>H+20)p.y=-20;
    ctx.beginPath(); ctx.arc(p.x,p.y,p.r,0,6.283);
    ctx.fillStyle=p.c+p.a+')'; ctx.fill();
  }
  ctx.beginPath();
  ctx.strokeStyle='rgba(94,160,220,0.12)';
  ctx.lineWidth=0.7;
  for(i=0;i<parts.length;i++){
    for(j=i+1;j<parts.length;j++){
      p=parts[i]; q=parts[j];
      dx=p.x-q.x; dy=p.y-q.y;
      if(dx*dx+dy*dy<11000){
        ctx.moveTo(p.x,p.y);
        ctx.lineTo(q.x,q.y);
      }
    }
  }
  ctx.stroke();
  canvasRafId=requestAnimationFrame(drawCanvas);
}
sizeCanvas(); drawCanvas();
window.addEventListener('mousemove',function(e){
  var r=canvas.getBoundingClientRect();
  mouse.x=e.clientX-r.left; mouse.y=e.clientY-r.top;
},{passive:true});
new IntersectionObserver(function(en){
  var active=en[0].isIntersecting;
  if(active && !canvasRunning){
    canvasRunning=true;
    if(!canvasRafId) canvasRafId=requestAnimationFrame(drawCanvas);
  } else if(!active){
    canvasRunning=false;
  }
}).observe(document.getElementById('home'));

/* ---------- Custom cursor ---------- */
if(isFine){
  var dot=document.getElementById('cursor-dot'), ring=document.getElementById('cursor-ring');
  var mx=innerWidth/2,my=innerHeight/2,rx=mx,ry=my;
  window.addEventListener('mousemove',function(e){
    mx=e.clientX; my=e.clientY;
    dot.style.transform='translate3d('+(mx-3.5)+'px,'+(my-3.5)+'px,0)';
  },{passive:true});
  (function loop(){
    rx+=(mx-rx)*.18; ry+=(my-ry)*.18;
    ring.style.transform='translate3d('+(rx-17)+'px,'+(ry-17)+'px,0)';
    requestAnimationFrame(loop);
  })();
  document.querySelectorAll('a,button,.service-card,select,input,textarea').forEach(function(el){
    el.addEventListener('mouseenter',function(){ ring.classList.add('grow'); });
    el.addEventListener('mouseleave',function(){ ring.classList.remove('grow'); });
  });
} else {
  document.getElementById('cursor-dot').style.display='none';
  document.getElementById('cursor-ring').style.display='none';
}

/* ---------- Nav / mobile menu / progress / anchors ---------- */
var nav=document.getElementById('site-nav'), progress=document.getElementById('scroll-progress'), backTop=document.getElementById('back-top');
function updateScrollIndicators(st, maxScroll){
  var ratio=maxScroll>0 ? Math.min(1, Math.max(0, st/maxScroll)) : 0;
  progress.style.transform='scaleX('+ratio+')';
  nav.classList.toggle('scrolled',st>40);
  backTop.classList.toggle('show',st>600);
}
if(lenis){
  lenis.on('scroll',function(e){
    updateScrollIndicators(e.scroll, e.limit);
  });
} else {
  window.addEventListener('scroll',function(){
    var st=window.scrollY||document.documentElement.scrollTop;
    var maxScroll=document.documentElement.scrollHeight-innerHeight;
    updateScrollIndicators(st, maxScroll);
  },{passive:true});
}

var menuBtn=document.getElementById('menu-btn'), mobileMenu=document.getElementById('mobile-menu');
function closeMobileMenu(){
  mobileMenu.classList.remove('open'); menuBtn.classList.remove('open');
  if(!document.body.classList.contains('locked') && lenis) lenis.start();
}
menuBtn.addEventListener('click',function(){
  var open=mobileMenu.classList.toggle('open');
  menuBtn.classList.toggle('open',open);
  if(open){
    if(lenis) lenis.stop();
    gsap.fromTo('#mobile-menu .mm-link',{y:36,opacity:0},{y:0,opacity:1,duration:.55,stagger:.08,ease:'power3.out',delay:.1});
  } else if(lenis && !document.body.classList.contains('locked')) lenis.start();
});

function scrollToTarget(sel){
  var el=document.querySelector(sel); if(!el) return;
  if(lenis) lenis.scrollTo(el,{offset:-64,duration:0.9});
  else el.scrollIntoView({behavior:'smooth'});
}
document.querySelectorAll('a[href^="#"]').forEach(function(a){
  a.addEventListener('click',function(e){
    var id=a.getAttribute('href');
    if(id.length>1 && document.querySelector(id)){ e.preventDefault(); closeMobileMenu(); scrollToTarget(id); }
  });
});
backTop.addEventListener('click',function(){
  if(lenis) lenis.scrollTo(0,{duration:0.9}); else window.scrollTo({top:0,behavior:'smooth'});
});

/* Active nav link */
function setActive(id){
  document.querySelectorAll('.nav-link').forEach(function(l){ l.classList.toggle('active',l.dataset.nav===id); });
}
['home','services','about','quote'].forEach(function(id){
  ScrollTrigger.create({trigger:'#'+id,start:'top 55%',end:'bottom 55%',
    onToggle:function(s){ if(s.isActive) setActive(id); }});
});

/* ---------- Anime-style connector line ---------- */
var glowPath=document.getElementById('connector-glow'),
    dimPath=document.getElementById('connector-dim'),
    comet=document.getElementById('comet'),
    traveler=document.getElementById('traveler'),
    connectorTween=null, pathLen=0, nodePts=[], nodeLens=[];

function smoothPath(pts){
  if(pts.length<2) return '';
  var d='M '+pts[0].x.toFixed(1)+' '+pts[0].y.toFixed(1);
  for(var i=0;i<pts.length-1;i++){
    var p0=pts[i-1]||pts[i], p1=pts[i], p2=pts[i+1], p3=pts[i+2]||p2;
    var c1x=p1.x+(p2.x-p0.x)/6, c1y=p1.y+(p2.y-p0.y)/6;
    var c2x=p2.x-(p3.x-p1.x)/6, c2y=p2.y-(p3.y-p1.y)/6;
    d+=' C '+c1x.toFixed(1)+' '+c1y.toFixed(1)+', '+c2x.toFixed(1)+' '+c2y.toFixed(1)+', '+p2.x.toFixed(1)+' '+p2.y.toFixed(1);
  }
  return d;
}
function renderConnector(p){
  if(!pathLen) return;
  glowPath.style.strokeDashoffset=pathLen*(1-p);
  var s=Math.max(0,Math.min(p*pathLen,pathLen));
  var pt=glowPath.getPointAtLength(s);
  var pt2=glowPath.getPointAtLength(Math.max(0,s-8));
  var ang=Math.atan2(pt.y-pt2.y,pt.x-pt2.x)*180/Math.PI;
  comet.style.transform='translate3d('+pt.x+'px,'+pt.y+'px,0) rotate('+ang+'deg)';
  traveler.style.transform='translate3d('+(pt.x-210)+'px,'+(pt.y-210)+'px,0)';
  nodePts.forEach(function(n,i){
    var lit=s>=nodeLens[i]-2;
    if(n.wasLit!==lit){
      n.wasLit=lit;
      n.el.classList.toggle('lit',lit);
      var card=n.row.querySelector('.service-card');
      if(card) card.classList.toggle('energized',lit);
    }
  });
}
function buildConnector(){
  var track=document.getElementById('services-track');
  if(!track) return;
  var nodes=Array.prototype.slice.call(track.querySelectorAll('.service-node'));
  var tr=track.getBoundingClientRect();
  nodePts=nodes.map(function(n){
    var r=n.getBoundingClientRect();
    return {x:r.left+r.width/2-tr.left, y:r.top+r.height/2-tr.top, el:n, row:n.closest('.service-row'), wasLit:false};
  });
  if(nodePts.length<2) return;
  var start={x:nodePts[0].x, y:Math.max(-30,nodePts[0].y-150)};
  var last=nodePts[nodePts.length-1];
  var end={x:last.x, y:last.y+150};
  var d=smoothPath([start].concat(nodePts,[end]));
  dimPath.setAttribute('d',d);
  glowPath.setAttribute('d',d);
  pathLen=glowPath.getTotalLength();
  glowPath.style.strokeDasharray=pathLen;
  glowPath.style.strokeDashoffset=pathLen;
  nodeLens=nodePts.map(function(p){
    var best=0,bd=1e12,s,q,dd;
    for(s=0;s<=pathLen;s+=6){
      q=glowPath.getPointAtLength(s);
      dd=(q.x-p.x)*(q.x-p.x)+(q.y-p.y)*(q.y-p.y);
      if(dd<bd){bd=dd;best=s;}
    }
    return best;
  });
  if(connectorTween){
    if(connectorTween.scrollTrigger) connectorTween.scrollTrigger.kill();
    connectorTween.kill();
  }
  var proxy={p:0};
  connectorTween=gsap.to(proxy,{p:1,ease:'none',
    scrollTrigger:{trigger:track,start:'top 78%',end:'bottom 62%',scrub:.5,invalidateOnRefresh:true},
    onUpdate:function(){ renderConnector(proxy.p); }});
  renderConnector(0);
}

/* ---------- Scroll animations ---------- */
gsap.utils.toArray('[data-reveal]').forEach(function(el){
  gsap.fromTo(el,{y:46,opacity:0},{y:0,opacity:1,duration:.95,ease:'power3.out',
    scrollTrigger:{trigger:el,start:'top 88%'}});
});
gsap.utils.toArray('.service-row').forEach(function(row){
  var side=row.dataset.side==='left'?-1:1;
  var wrap=row.querySelector('.sc-card-wrap');
  gsap.fromTo(wrap,{x:70*side,opacity:0,rotateY:9*side},{x:0,opacity:1,rotateY:0,duration:1.1,ease:'power3.out',
    scrollTrigger:{trigger:row,start:'top 86%'}});
  gsap.fromTo(row.querySelector('.service-node'),{scale:0},{scale:1,duration:.6,ease:'back.out(2.2)',
    scrollTrigger:{trigger:row,start:'top 84%'}});
});
/* Parallax layers */
gsap.utils.toArray('.svc-orb,.svc-shape,.about-orb').forEach(function(el){
  var sp=parseFloat(el.dataset.speed||'-15');
  gsap.to(el,{yPercent:sp,ease:'none',force3D:true,
    scrollTrigger:{trigger:el.closest('section'),start:'top bottom',end:'bottom top',scrub:0.4}});
});
/* Hero parallax out */
gsap.to('#hero-content',{yPercent:-16,opacity:.2,ease:'none',force3D:true,
  scrollTrigger:{trigger:'#home',start:'top top',end:'bottom top',scrub:0.3}});
gsap.to('.hero-orb-1',{yPercent:34,ease:'none',force3D:true,scrollTrigger:{trigger:'#home',start:'top top',end:'bottom top',scrub:0.4}});
gsap.to('.hero-orb-2',{yPercent:-26,ease:'none',force3D:true,scrollTrigger:{trigger:'#home',start:'top top',end:'bottom top',scrub:0.4}});

/* Stats counters */
gsap.utils.toArray('.stat-num').forEach(function(el){
  var target=parseFloat(el.dataset.count), suffix=el.dataset.suffix||'';
  var obj={v:0}, isFloat=target%1!==0;
  gsap.to(obj,{v:target,duration:2.1,ease:'power2.out',
    scrollTrigger:{trigger:el,start:'top 90%'},
    onUpdate:function(){ el.textContent=(isFloat?obj.v.toFixed(1):Math.round(obj.v))+suffix; }});
});

/* Card tilt + spotlight */
if(isFine){
  document.querySelectorAll('.service-card').forEach(function(card){
    card.addEventListener('mousemove',function(e){
      var r=card.getBoundingClientRect();
      var px=(e.clientX-r.left)/r.width, py=(e.clientY-r.top)/r.height;
      card.style.transform='rotateX('+((py-.5)*-8).toFixed(2)+'deg) rotateY('+((px-.5)*10).toFixed(2)+'deg) translateZ(6px)';
      card.style.setProperty('--mx',(px*100)+'%');
      card.style.setProperty('--my',(py*100)+'%');
    });
    card.addEventListener('mouseleave',function(){ card.style.transform=''; });
  });
}

/* Card -> prefill quote form */
document.querySelectorAll('[data-service-quote]').forEach(function(btn){
  btn.addEventListener('click',function(){
    var sel=document.getElementById('q-service');
    sel.value=btn.dataset.serviceQuote;
    scrollToTarget('#quote');
    gsap.fromTo(sel,{boxShadow:'0 0 0 0 rgba(251,191,36,.55)'},{boxShadow:'0 0 0 7px rgba(251,191,36,0)',duration:1.2,delay:.7});
  });
});

/* ---------- Quote form ---------- */
var form=document.getElementById('quote-form'),
    success=document.getElementById('quote-success'),
    submitBtn=document.getElementById('q-submit');
var validators=[
  {el:document.getElementById('q-name'),   test:function(v){return v.trim().length>=2;}},
  {el:document.getElementById('q-email'),  test:function(v){return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v.trim());}},
  {el:document.getElementById('q-company'),test:function(v){return v.trim().length>=2;}},
  {el:document.getElementById('q-desc'),   test:function(v){return v.trim().length>=10;}}
];
validators.forEach(function(f){
  f.el.addEventListener('input',function(){ f.el.closest('.field').classList.remove('invalid'); });
});
form.addEventListener('submit',function(e){
  e.preventDefault();
  var ok=true, first=null;
  validators.forEach(function(f){
    var valid=f.test(f.el.value);
    f.el.closest('.field').classList.toggle('invalid',!valid);
    if(!valid){ ok=false; if(!first) first=f.el; }
  });
  if(!ok){
    if(first) first.focus();
    gsap.fromTo(submitBtn,{x:0},{x:-7,duration:.06,repeat:5,yoyo:true,clearProps:'x'});
    return;
  }
  submitBtn.disabled=true;
  submitBtn.classList.add('loading');
  submitBtn.querySelector('.btn-label').textContent='Transmitting...';
  setTimeout(function(){
    gsap.to(form,{opacity:0,y:-16,duration:.4,onComplete:function(){
      form.style.display='none';
      success.classList.remove('hidden');
      success.classList.add('show');
      gsap.fromTo(success,{opacity:0,scale:.92},{opacity:1,scale:1,duration:.6,ease:'back.out(1.6)'});
      burst();
    }});
  },1500);
});
function burst(){
  var zone=document.getElementById('burst-zone');
  var colors=['#2dd4bf','#38bdf8','#fbbf24','#5eead4','#fde68a'];
  for(var i=0;i<26;i++){
    var s=document.createElement('span');
    s.className='burst-p';
    s.style.background=colors[i%colors.length];
    zone.appendChild(s);
    var ang=Math.random()*6.283, dist=60+Math.random()*90;
    gsap.fromTo(s,{x:0,y:0,opacity:1,scale:Math.random()*.8+.5},
      {x:Math.cos(ang)*dist,y:Math.sin(ang)*dist,opacity:0,scale:.1,duration:1+Math.random()*.6,ease:'power2.out',
       onComplete:(function(el){return function(){el.remove();};})(s)});
  }
}
document.getElementById('q-again').addEventListener('click',function(){
  gsap.to(success,{opacity:0,duration:.3,onComplete:function(){
    success.classList.add('hidden'); success.classList.remove('show'); success.style.opacity='';
    form.reset();
    form.style.display=''; 
    gsap.fromTo(form,{opacity:0,y:16},{opacity:1,y:0,duration:.5});
    submitBtn.disabled=false;
    submitBtn.classList.remove('loading');
    submitBtn.querySelector('.btn-label').textContent='Send Request';
  }});
});

/* ---------- Marquee duplicate for seamless loop ---------- */
var mq=document.getElementById('marquee-track');
mq.innerHTML+=mq.innerHTML;

/* ---------- Build connector at the right moments ---------- */
window.addEventListener('load',function(){ buildConnector(); ScrollTrigger.refresh(); });
if(document.fonts && document.fonts.ready) document.fonts.ready.then(function(){ buildConnector(); });
var rzT;
window.addEventListener('resize',function(){
  clearTimeout(rzT);
  rzT=setTimeout(function(){ sizeCanvas(); buildConnector(); ScrollTrigger.refresh(); },260);
});
setTimeout(buildConnector,800);

})();
});

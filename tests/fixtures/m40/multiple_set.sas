/* M40.2 - Statements SET multiples + SET nu.
   1) Deux SET dans la meme etape : lecture PARALLELE (1 obs de chaque flux
      par iteration), fin au PREMIER EOF - a (3 obs) x b (2 obs) => 2 obs.
      La NOTE de lecture compte 3 obs lues sur A (la 3e est chargee avant
      que l'EOF de B ne stoppe l'etape) et 2 sur B.
   2) END= par site : le flag du petit flux s'allume sur SA derniere obs,
      independamment de l'autre site.
   3) Auto-retain par site : un SET conditionnel (if _n_=1) charge une valeur
      qui persiste sur toutes les iterations suivantes.
   4) SET nu (set;) : relit _LAST_, le dataset le plus recemment cree. */

data work.a;
  input x;
  datalines;
1
2
3
;
run;

data work.b;
  input y;
  datalines;
10
20
;
run;

data work.pairs;
  set work.a;
  set work.b;
run;

title "Deux SET : obs appariees, fin au premier EOF (2 obs)";
proc print data=work.pairs noobs;
run;
title;

data work.flags;
  set work.a end=ea;
  set work.b end=eb;
  fa = ea;
  fb = eb;
  drop x y;
run;

title "END= par site : eb s'allume sur la 2e obs, ea jamais (EOF avant sa derniere)";
proc print data=work.flags noobs;
run;
title;

data work.totals;
  total = 99;
run;

data work.scored;
  set work.b;
  if _n_ = 1 then set work.totals;
  share = y / total;
run;

title "SET conditionnel : total charge une fois puis auto-retenu";
proc print data=work.scored noobs;
run;
title;

data work.again;
  set;
run;

title "SET nu : relit _LAST_ (work.scored)";
proc print data=work.again noobs;
run;
title;

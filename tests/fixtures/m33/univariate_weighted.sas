/* M33.2 : PROC UNIVARIATE — quantiles et Extreme Observations PONDERES, plus
   le rendu PROBPLOT routes via ODS GRAPHICS (note "image deferred" en build
   par defaut).

   Donnees (verifiables a la main) : x=[1,2,3,4], w=[1,2,3,4].
     Poids total W = 1+2+3+4 = 10 ; poids cumules W_i : 1, 3, 6, 10.
   Quantiles ponderes (Definition 5 ponderee : cible t=p*W ; premier i avec
   W_i>=t ; si W_i==t exactement -> moyenne x(i),x(i+1), sinon x(i)) :
     100% Max = 4 ; 75% Q3 (t=7.5 -> W_4=10) = 4 ; 50% Median (t=5 -> W_3=6) = 3 ;
     25% Q1 (t=2.5 -> W_2=3) = 2 ; 10% (t=1.0 == W_1) = (1+2)/2 = 1.5 ;
     5% (t=0.5 -> W_1=1) = 1 ; 0% Min = 1.
   Moments ponderes : SumWgt=10, Sum Obs=Swx=1+4+9+16=30, Mean=30/10=3,
     CSS_w=Sw(x-3)^2 = 1*4+2*1+3*0+4*1 = 10, Var=CSS_w/(n-1)=10/3.
   Extreme Observations : valeurs brutes 1,2,3,4 aux obs 1..4 (non ponderees). */

data wq;
  x = 1; w = 1; output;
  x = 2; w = 2; output;
  x = 3; w = 3; output;
  x = 4; w = 4; output;
run;

title 'Weighted quantiles and extremes (weight w)';
proc univariate data=wq;
  var x;
  weight w;
run;

/* PROBPLOT sous ODS GRAPHICS ON : en build par defaut (sans --features
   graphics) chaque trace emet la note partagee "image deferred". */
ods graphics on;
title 'Weighted univariate with a probability plot';
proc univariate data=wq;
  var x;
  weight w;
  probplot x;
run;
ods graphics off;

title;

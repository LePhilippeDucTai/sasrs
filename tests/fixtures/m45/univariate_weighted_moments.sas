/* M45.1 - PROC UNIVARIATE : Skewness / Kurtosis PONDERES (SAS, VARDEF=DF).

   Avec z_i = sqrt(w_i) * (x_i - xbar_w) / s_w, ou xbar_w = Sw*x / Sw et
   s_w = sqrt(Sw(x-xbar_w)^2 / (n-1)) :
     g1 = n/((n-1)(n-2))             * S z_i^3
     g2 = [n(n+1)/((n-1)(n-2)(n-3))] * S z_i^4 - 3(n-1)^2/((n-2)(n-3))
   n est le NOMBRE D'OBSERVATIONS utilisables, pas la somme des poids.

   1) Oracle calculable a la main : x=[1,2,3,4], w=[1,2,3,4].
        Sw = 10 ; Swx = 1+4+9+16 = 30 ⇒ xbar_w = 3
        CSS_w = 1*4 + 2*1 + 3*0 + 4*1 = 10 ⇒ s_w^2 = 10/3 (n-1 = 3)
        S w^{3/2}(x-3)^3 = -8 - 2*sqrt(2) + 0 + 8 = -2*sqrt(2)
          g1 = 4/((3)(2)) * (-2*sqrt(2)) / (10/3)^{3/2} = -0.309838668
        S w^2(x-3)^4 = 16 + 4 + 0 + 16 = 36 ; s_w^4 = 100/9
          g2 = [4*5/((3)(2)(1))] * 36/(100/9) - 3*(3)^2/((2)(1))
             = 10.8 - 13.5 = -2.7   (exact)

   2) ORACLE DE REDUCTION : le meme x avec w = 1 partout, puis le meme x SANS
      instruction WEIGHT. sqrt(1) = 1, donc les formules ponderees redonnent
      exactement les non ponderees : les deux blocs Moments doivent afficher
      des Skewness / Kurtosis IDENTIQUES (comparaison directe dans ce snapshot).
      x=[1,2,3,4] est symetrique ⇒ Skewness = 0, Kurtosis = -1.2.

   3) Jeu pondere SYMETRIQUE : x=[1,2,3], w=[2,1,2].
        xbar_w = (2*1 + 1*2 + 2*3)/5 = 10/5 = 2
        S w^{3/2}(x-2)^3 = 2^1.5*(-1) + 1*0 + 2^1.5*(+1) = 0  ⇒ Skewness = 0
      n = 3 < 4 ⇒ Kurtosis absente (missing), comme le chemin non pondere. */

data w1;
  x = 1; w = 1; output;
  x = 2; w = 2; output;
  x = 3; w = 3; output;
  x = 4; w = 4; output;
run;

title 'Weighted skewness/kurtosis: hand-computed oracle (g1=-0.309838668, g2=-2.7)';
proc univariate data=w1;
  var x;
  weight w;
run;

data unit;
  x = 1; w = 1; output;
  x = 2; w = 1; output;
  x = 3; w = 1; output;
  x = 4; w = 1; output;
run;

title 'Unit weights: weighted formulas must reduce to the unweighted ones';
proc univariate data=unit;
  var x;
  weight w;
run;

title 'Same data, no WEIGHT statement: Skewness/Kurtosis must match the block above';
proc univariate data=unit;
  var x;
run;

data sym;
  x = 1; w = 2; output;
  x = 2; w = 1; output;
  x = 3; w = 2; output;
run;

title 'Weighted symmetric distribution: Skewness = 0 (Kurtosis needs n>=4)';
proc univariate data=sym;
  var x;
  weight w;
run;

title;

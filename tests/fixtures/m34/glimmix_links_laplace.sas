/* M34.8 : PROC GLIMMIX — LINK=PROBIT/CLOGLOG (no random → cross-check LOGISTIC)
   and METHOD=LAPLACE (Normal + random intercept → cross-check MIXED ML).

   Saturated 2-point binary model (m26 counts): fitted p(x=1)=0.8, p(x=0)=0.285714
   exactly ⇒ closed-form estimates, identical to PROC LOGISTIC's link fits:
     PROBIT  : Intercept = Phi^-1(0.285714) = -0.5659 ; x = 1.4076
     CLOGLOG : Intercept = ln(-ln(0.714286)) = -1.0892 ; x = 1.5651

   LAPLACE (Normal+Identity+random intercept) ≡ MIXED METHOD=ML on the balanced
   data A=(1,3), B=(5,7): grand mean 4 ⇒ Intercept = 4.0000; the variance
   components match MIXED ML (Var(subj intercept) and Residual). */
title 'PROC GLIMMIX PROBIT (binary, no random) — cross-check LOGISTIC';
data counts;
  input y x count;
datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;
proc glimmix data=counts;
  model y(event='1') = x / dist=binary link=probit solution;
  freq count;
run;

title 'PROC GLIMMIX CLOGLOG (binary, no random)';
proc glimmix data=counts;
  model y(event='1') = x / dist=binary link=cloglog solution;
  freq count;
run;

title 'PROC GLIMMIX METHOD=LAPLACE (Normal + random intercept) — cross-check MIXED ML';
data balanced;
  input subj $ y;
datalines;
A 1
A 3
B 5
B 7
;
proc glimmix data=balanced method=laplace;
  class subj;
  model y = / dist=normal link=identity solution;
  random intercept / subject=subj type=vc;
run;
title;

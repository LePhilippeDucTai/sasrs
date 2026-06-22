/* M34.6 : PROC LOGISTIC — LINK=PROBIT and LINK=CLOGLOG (non-logit links).
   Saturated 2-point model (m26 counts), so fitted p(x=1)=0.8, p(x=0)=0.285714
   exactly, and the estimates are closed-form (continuous x coded 0/1, modelling
   P(y=1) via DESCENDING):

   PROBIT:  Intercept = Phi^-1(0.285714) = -0.56595
            x        = Phi^-1(0.8) - Phi^-1(0.285714) = 0.84162+0.56595 = 1.40757
   CLOGLOG: eta = ln(-ln(1-p)); Intercept = ln(-ln(0.714286)) = -1.08924
            x = ln(-ln(0.2)) - (-1.08924) = 0.47588 + 1.08924 = 1.56512
   No Odds Ratio table is printed for non-logit links. */
title 'PROC LOGISTIC PROBIT link';
data counts;
  input y x count;
  datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;
proc logistic data=counts;
  model y(descending) = x / link=probit;
  freq count;
run;

title 'PROC LOGISTIC CLOGLOG link';
proc logistic data=counts;
  model y(descending) = x / link=cloglog;
  freq count;
run;
title;

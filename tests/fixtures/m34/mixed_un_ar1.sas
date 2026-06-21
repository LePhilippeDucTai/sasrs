/* M34.8 : PROC MIXED — REPEATED TYPE=UN and TYPE=AR(1) covariance structures.
   Four subjects, two repeated measures each (balanced):
     A=(1,3)  B=(3,1)  C=(5,7)  D=(7,5)
   Time-1 mean = Time-2 mean = grand mean = 4.

   UN oracle (METHOD=ML → MLE = sample covariance /N, N=4):
     Var(t1)=((−3)²+(−1)²+1²+3²)/4 = 20/4 = 5  → UN(1,1)=5
     Var(t2)= same by symmetry              5  → UN(2,2)=5
     Cov   =((−3)(−1)+(−1)(−3)+(1)(3)+(3)(1))/4 = 12/4 = 3 → UN(2,1)=3
     Intercept (GLS, intercept-only mean) = grand mean = 4.

   AR(1): structural — Covariance Parameter Estimates list AR(1)=ρ (|ρ|<1) and
   Residual=σ²>0; both finite, optimizer converged. */
title 'PROC MIXED REPEATED TYPE=UN (ML)';
data rep;
  input subj $ time y;
datalines;
A 1 1
A 2 3
B 1 3
B 2 1
C 1 5
C 2 7
D 1 7
D 2 5
;
proc mixed data=rep method=ml;
  class subj time;
  model y = / solution;
  repeated time / subject=subj type=un;
run;

title 'PROC MIXED REPEATED TYPE=AR(1) (REML)';
proc mixed data=rep;
  class subj time;
  model y = / solution;
  repeated time / subject=subj type=ar(1);
run;
title;

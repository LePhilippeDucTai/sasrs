/* M34.4 : PROC REG — SELECTION= (FORWARD, BACKWARD) and multiple MODEL
   statements, on sashelp.class. Candidates: height, age (both correlated with
   weight; height is the stronger predictor).

   Oracle reasoning:
     - FORWARD (slentry=0.50 default): height has the largest enter-F and enters
       at step 1; age is then tested and enters too (its partial p ≤ 0.50).
       Final model = height age, fit as ordinary 2-regressor OLS.
     - BACKWARD (slstay=0.10 default): starts with {height, age}; the variable
       with the largest remove-p is dropped if that p > 0.10. age's partial
       contribution given height is weak, so age is eliminated, leaving height.
     - Two MODEL statements in one PROC are labelled MODEL1 then MODEL2. */
libname d 'data';

title 'PROC REG FORWARD selection: weight = height age';
proc reg data=d.class;
  model weight = height age / selection=forward;
run;

title 'PROC REG BACKWARD selection: weight = height age';
proc reg data=d.class;
  model weight = height age / selection=backward;
run;

title 'PROC REG two MODEL statements';
proc reg data=d.class;
  model weight = height;
  model weight = height age;
run;

title;

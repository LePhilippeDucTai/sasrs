/* M11: CALL SYMPUT in one step, the value read in the NEXT step (interleaving). */
data seed;
  x = 42;
  call symput('answer', '42');
run;

data use;
  v = &answer;
run;

title 'CALL SYMPUT feeds the next step';
proc print data=use;
  var v;
run;

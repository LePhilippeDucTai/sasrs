data t;
  x=1; y=2; output;
  x=2; y=3; output;
  x=3; y=5; output;
run;
proc reg data=t;
  model y=x;
  reweight x < 2;
run;

title 'PROC IML: basic matrix operations and control flow';

proc iml;
  /* Creation and basic operations */
  a = {1 2, 3 4};
  b = {5 6, 7 8};
  c = a * b;
  at = a';
  h = a # b;
  nr = nrow(a);
  nc = ncol(a);
  print a b c at h nr nc;

  /* Statistical functions */
  x = {2 4 6 8 10};
  s = sum(x);
  m = mean(x);
  st = std(x);
  print x s m st;

  /* Control flow */
  total = {0};
  do i = 1 to 5;
    total = total + i;
  end;
  if total > {10} then big = {1}; else big = {0};
  print total big;
quit;

title;

/* ARRAY + DO itératif — motif documenté ARRAY Statement. */
libname ind 'data';

data scored;
  set ind.raw;
  array m{3} m1-m3;
  do i = 1 to 3;
    if m{i} = . then m{i} = 0;
  end;
  total = sum(m1, m2, m3);
  drop i;
run;

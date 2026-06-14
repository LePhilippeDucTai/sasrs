/* M16: constructions étape DATA — SELECT, arrays, DO OVER, SET options, UPDATE/MODIFY, LINK/RETURN/GOTO, RETAIN _ALL_ */

/* M16.1: SELECT/WHEN/OTHERWISE */
data work.select_test;
  do val = 1, 2, 3, 5, 10;
    select(val);
      when(1, 2, 3) grade = 'low';
      when(5) grade = 'mid';
      otherwise grade = 'high';
    end;
    output;
  end;
run;

/* M16.2: Multi-dimensional arrays + initial values */
data work.array_test;
  array matrix{2, 3} (1, 2, 3, 4, 5, 6);  /* 2x3 with initial values */
  array names{3} $10 _temporary_;
  names{1} = 'Alice';
  names{2} = 'Bob';
  names{3} = 'Carol';

  /* Test DIM, HBOUND, LBOUND */
  dims = dim(matrix);
  h1 = hbound(matrix, 1);
  h2 = hbound(matrix, 2);
  l1 = lbound(matrix, 1);

  /* Access multi-D element */
  elem_2_2 = matrix{2, 2};

  output;
run;

/* M16.3: DO OVER + DO with value lists + RETAIN with date */
data work.do_test;
  retain date_var 21710d;  /* 2020-01-01 */

  array nums{3} n1-n3;
  n1 = 10; n2 = 20; n3 = 30;

  do over nums;
    nums = nums * 2;  /* Double each element */
  end;

  /* DO with explicit value list */
  do i = 1, 3, 5, 7;
    output;
  end;
run;

/* M16.4: SET with END=, NOBS=, POINT= */
data work.set_test;
  set work.select_test(rename=(val=id)) end=eof nobs=n;
  obs_num = _n_;
  is_last = eof;
  total = n;
  output;
run;

/* Create a small dataset for UPDATE/MODIFY testing */
data work.master;
  input id val;
  datalines;
1 100
2 200
3 300
;
run;

data work.trans;
  input id newval;
  datalines;
1 150
3 350
;
run;

/* M16.5: UPDATE (master/transaction) */
data work.updated;
  update work.master work.trans key=id;
  output;
run;

/* M16.6: LINK/RETURN, GOTO, labels, RETAIN _ALL_ */
data work.control_test;
  retain _all_;  /* Retain all variables */
  input x;

  link calc;
  goto skip_error;

  error_section: y = -1;

  skip_error:
  output;
  stop;

  calc: y = x * 2; return;

  datalines;
5
10
15
;
run;

/* Final output */
title 'M16 Constructions — SELECT, Arrays, DO OVER, SET Options, UPDATE, LINK/GOTO';
proc print data=work.control_test;
run;

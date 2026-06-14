/* M15.5 : Random number functions — RAND, RANUNI, RANNOR, RANEXP,    */
/* RANBIN, CALL STREAMINIT.                                             */

data work.test;
  /* Initialize RNG stream */
  call streaminit(12345);

  /* RANUNI — uniform [0,1) */
  u1 = ranuni();
  u2 = ranuni();

  /* RANNOR — standard normal */
  n1 = rannor();
  n2 = rannor();

  /* RANEXP — exponential(1) */
  e1 = ranexp();

  /* RANBIN — binomial */
  b1 = ranbin(0.5, 10);      /* 10 trials, p=0.5 */
  b2 = ranbin(0.3, 20);      /* 20 trials, p=0.3 */

  /* RAND — generic */
  r_unif = rand('UNIFORM');  /* Should be ~[0,1) */
  r_norm = rand('NORMAL');   /* Should be ~N(0,1) */
  r_exp = rand('EXPONENTIAL');  /* Should be >0 */

  output;
run;

title 'M15.5 Random Number Functions';
proc print data=work.test;
run;

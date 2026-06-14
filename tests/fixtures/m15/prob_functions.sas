/* M15.4 : Probability functions — PROBNORM, PROBT, PROBF, PROBCHI,  */
/* PROBBETA, PROBGAM, CDF, PDF, QUANTILE, SDF, LOGCDF, PROBBNML,       */
/* POISSON.                                                              */

data work.test;
  /* PROBNORM — standard normal CDF */
  pn_0 = probnorm(0);        /* Should be 0.5 */
  pn_1 = probnorm(1.0);      /* Should be ~0.8413 */
  pn_neg1 = probnorm(-1.0);  /* Should be ~0.1587 */

  /* PROBT — Student's t CDF */
  pt_0_1 = probt(0, 1);      /* Should be 0.5 */
  pt_1_5 = probt(1.0, 5);    /* Should be ~0.825 */
  pt_neg1_5 = probt(-1.0, 5);  /* Should be ~0.175 */

  /* PROBF — F CDF */
  pf_1_1_1 = probf(1.0, 1, 1);  /* Should be ~0.5 */
  pf_2_3_5 = probf(2.0, 3, 5);  /* Should be ~0.724 */

  /* PROBCHI — chi-square CDF */
  pc_0_1 = probchi(0, 1);    /* Should be 0 */
  pc_1_1 = probchi(1.0, 1);  /* Should be ~0.683 */
  pc_4_1 = probchi(4.0, 1);  /* Should be ~0.954 */

  /* PROBBETA — Beta CDF */
  pb_0_5_1_1 = probbeta(0.5, 1, 1);  /* Should be 0.5 (uniform) */
  pb_0_5_2_2 = probbeta(0.5, 2, 2);  /* Should be 0.5 */

  /* PROBGAM — Gamma CDF */
  pg_1_1 = probgam(1.0, 1);  /* Should be ~0.632 */
  pg_2_1 = probgam(2.0, 1);  /* Should be ~0.865 */

  /* PROBBNML — Binomial CDF */
  pbn_1_5_2 = probbnml(0.5, 5, 2);  /* P(X<=2) for n=5, p=0.5 */
  pbn_1_5_5 = probbnml(0.5, 5, 5);  /* P(X<=5) for n=5, p=0.5 = 1.0 */

  /* POISSON — Poisson CDF */
  pp_1_2 = poisson(1.0, 2);  /* P(X<=2) for lambda=1 */
  pp_2_1 = poisson(2.0, 1);  /* P(X<=1) for lambda=2 */

  output;
run;

title 'M15.4 Probability Functions';
proc print data=work.test;
run;

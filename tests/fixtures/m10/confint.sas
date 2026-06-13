/* M10 : MEANS intervalles de confiance (CLM/LCLM/UCLM) sur height.
   n=19, mean=62.336842, std=5.127075, stderr=std/sqrt(19)=1.176228,
   t(0.975,18)=2.100922 -> demi-largeur 2.471, CI ~ [59.866, 64.808]. */
libname d 'data';

title '95% confidence limits for mean height';
proc means data=d.class n mean std stderr lclm uclm;
  var height;
run;

// Net types and switch-like primitives: tri0/tri1, supply0/supply1, wand/wor, pullup/pulldown,
// bufif/notif/nmos/pmos/cmos, drive strengths (parsed, not modelled) and gate delays.
module tb;
  reg en1 = 0, en2 = 0, d1 = 1, d2 = 0, c = 0;
  tri0 t0;
  tri1 t1;
  supply0 gnd;
  supply1 vdd;
  wand wa;
  wor wo;
  wire pu, pd, b1, b0, n1, n0, nm, pm, cm;

  assign t0 = en1 ? d1 : 1'bz;
  assign t1 = en1 ? d2 : 1'bz;
  assign wa = en1 ? d1 : 1'bz;
  assign wa = en2 ? d2 : 1'bz;
  assign wo = en1 ? d1 : 1'bz;
  assign wo = en2 ? d2 : 1'bz;
  assign (strong0, weak1) pu = en1 ? d2 : 1'bz;
  pullup (pu);
  pulldown p0 (pd);
  bufif1 (b1, d1, c);
  bufif0 #(1,2) (b0, d1, c);
  notif1 (n1, d1, c);
  notif0 (n0, d1, c);
  nmos (nm, d1, c);
  pmos (pm, d1, c);
  cmos (cm, d1, c, ~c);
  initial begin
    #3 $display("t0=%b t1=%b gnd=%b vdd=%b wa=%b wo=%b pu=%b pd=%b | %b %b %b %b %b %b %b", t0, t1, gnd, vdd, wa, wo, pu, pd, b1, b0, n1, n0, nm, pm, cm);
    en1 = 1; c = 1;
    #3 $display("t0=%b t1=%b gnd=%b vdd=%b wa=%b wo=%b pu=%b pd=%b | %b %b %b %b %b %b %b", t0, t1, gnd, vdd, wa, wo, pu, pd, b1, b0, n1, n0, nm, pm, cm);
    en2 = 1; c = 1'bx;
    #3 $display("t0=%b t1=%b gnd=%b vdd=%b wa=%b wo=%b pu=%b pd=%b | %b %b %b %b %b %b %b", t0, t1, gnd, vdd, wa, wo, pu, pd, b1, b0, n1, n0, nm, pm, cm);
    d2 = 1; en1 = 0;
    #3 $display("t0=%b t1=%b wa=%b wo=%b pu=%b", t0, t1, wa, wo, pu);
  end
endmodule

// randomize() with an `inside` range and bounds on the same variable (IEEE 1800-2017 18.5.6,
// 18.7): the value is in both, also with a constraint on part of another variable; through
// a handle of a type parameter's type, the names in `with` are the object's members.
class S;
  rand logic [31:0] a;
  rand bit [31:0] b;
  rand int t;
  constraint c { t inside {[1:1024]}; }
  constraint d { b[1:0] == 0; }
endclass
class item; rand logic m; rand int n; endclass
class seq #(type REQ = item);
  REQ req;
  function int go();
    req = new;
    return req.randomize() with { m == 1; n inside {[5:7]}; n > 6; };
  endfunction
endclass
module top;
  int fails = 0;
  initial begin
    S s = new;
    seq #(item) q = new;
    repeat (20) begin
      if (!s.randomize() with { t == 128; a inside {[32'h0101_0000:32'h0103_0000]}; }) fails++;
      if (s.t != 128 || s.b[1:0] != 0 || s.a < 32'h0101_0000 || s.a > 32'h0103_0000) fails++;
      if (!s.randomize() with { t > 1000; }) fails++;
      if (s.t <= 1000 || s.t > 1024) fails++;
    end
    if (!q.go() || q.req.m != 1 || q.req.n != 7) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule

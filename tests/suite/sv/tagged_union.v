// expected
// tagged unions (IEEE 1800-2017 7.3.2, 11.9) and pattern matching (12.6): the tag above
// the widest member, each member right-justified; case/if/?: with matches
module top;
  typedef union tagged packed {
    struct packed { bit [4:0] reg1, reg2, regd; } Add;
    union tagged packed {
      bit [9:0] JmpU;
      struct packed { bit [1:0] cc; bit [9:0] addr; } JmpC;
    } Jmp;
  } Instr;
  typedef union tagged { void Invalid; int Valid; } VInt;

  Instr i;
  VInt v;
  int n;

  function automatic string show(Instr x);
    case (x) matches
      tagged Add '{.r1, .r2, .rd} &&& (rd != 0): return $sformatf("add %0d %0d %0d", r1, r2, rd);
      tagged Add .*: return "add to r0";
      tagged Jmp (tagged JmpU .a): return $sformatf("jmp %0d", a);
      tagged Jmp (tagged JmpC '{2'd0, .a}): return $sformatf("jmp never %0d", a);
      tagged Jmp (tagged JmpC '{.c, .a}): return $sformatf("jmp if %0d to %0d", c, a);
    endcase
  endfunction

  task automatic nested(Instr x);
    case (x) matches
      tagged Jmp .j:
        case (j) matches
          tagged JmpU .a: $display("nested jmp %0d", a);
          tagged JmpC '{.c, .a}: $display("nested jmpc %0d %0d", c, a);
        endcase
      default: $display("nested other");
    endcase
  endtask

  initial begin
    $display("%0d %0d", $bits(Instr), $bits(VInt));
    i = tagged Add '{1, 2, 3};
    $display("%b %s", i, show(i));
    nested(i);
    i = tagged Add '{reg1: 4, reg2: 5, regd: 0};
    $display("%s", show(i));
    i = tagged Jmp (tagged JmpC '{2, 9});
    $display("%b %s", i, show(i));
    nested(i);
    i = tagged Jmp (tagged JmpC '{0, 7});
    $display("%s", show(i));
    i = tagged Jmp (tagged JmpU 10'd700);
    $display("%b %s", i, show(i));
    nested(i);
    $display("%0d", i.Jmp.JmpU);

    // if with matches, &&& and else
    i = tagged Jmp (tagged JmpC '{2, 9});
    if (i matches tagged Jmp (tagged JmpC '{.c, .a}) &&& c == 2) $display("if %0d", a);
    else $display("else");
    if (i matches tagged Jmp (tagged JmpC '{.c, .a}) &&& c == 1) $display("if %0d", a);
    else $display("else");
    if (i matches tagged Add .*) $display("add");
    else $display("not add");

    // a conditional expression: the pattern variables in the true branch
    n = i matches tagged Jmp (tagged JmpC '{.c, .a}) ? a + c : -1;
    $display("%0d", n);
    n = i matches tagged Add '{.x, .*, .*} ? x : -1;
    $display("%0d", n);

    // an unpacked tagged union with a void member
    v = tagged Invalid;
    if (v matches tagged Valid .k) $display("valid %0d", k);
    else $display("invalid");
    v = tagged Valid (-5);
    if (v matches tagged Valid .k) $display("valid %0d", k);
    $display("%0d", v.Valid + 1);

    // casez in a case matches
    i = tagged Add '{5'b10110, 0, 1};
    casez (i) matches
      tagged Add '{5'b1?1?0, .*, .*}: $display("casez hit");
      default: $display("casez miss");
    endcase
    case (i) matches
      tagged Add '{5'b1?1?0, .*, .*}: $display("case hit");
      default: $display("case miss");
    endcase
  end
endmodule
